use super::*;

// =========================================================================
// 1. PHP Extraction
// =========================================================================

pub(super) fn extract_php(ctx: &mut ExtractCtx<'_>, root: Node<'_>) {
    let mut class_stack: Vec<(String, String)> = Vec::new(); // (kind, name)
    let mut fn_depth = 0usize;
    let mut this_props: BTreeMap<String, String> = BTreeMap::new();
    let mut var_types: BTreeMap<String, String> = BTreeMap::new();

    walk_php(
        ctx,
        root,
        0,
        &mut class_stack,
        &mut fn_depth,
        &mut this_props,
        &mut var_types,
    );
}

fn walk_php(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    depth: usize,
    class_stack: &mut Vec<(String, String)>,
    fn_depth: &mut usize,
    this_props: &mut BTreeMap<String, String>,
    var_types: &mut BTreeMap<String, String>,
) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    let kind = node.kind();
    let cur_class = class_stack.last().map(|(_, n)| n.clone());

    match kind {
        "namespace_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                ctx.namespace = Some(ctx.text(name_node).trim().trim_matches('\\').to_owned());
            }
        }
        "namespace_use_declaration" => {
            extract_php_namespace_use(ctx, node);
            return;
        }
        "use_declaration" => {
            // Trait use inside class/trait/enum.
            if let Some(owner) = cur_class.as_deref() {
                let line = start_line(node);
                let evidence = ctx.text(node);
                for i in 0..node.child_count() {
                    if let Some(ch) = node.child(i)
                        && matches!(ch.kind(), "name" | "qualified_name")
                    {
                        let trait_path = ctx.text(ch).trim_start_matches('\\').to_owned();
                        let short = trait_path
                            .rsplit('\\')
                            .next()
                            .unwrap_or(&trait_path)
                            .to_owned();
                        ctx.push_import(AstImport {
                            path: trait_path.clone(),
                            symbols: vec![short.clone()],
                            alias: None,
                            is_wildcard: false,
                            is_reexport: false,
                            reexport_visibility: None,
                            kind: "trait_use".to_owned(),
                            line,
                            evidence: clip_str(evidence, 160),
                        });
                        ctx.push_inheritance(owner, &trait_path, "uses", line, evidence);
                    }
                }
            }
            return;
        }
        "include_expression"
        | "include_once_expression"
        | "require_expression"
        | "require_once_expression" => {
            let line = start_line(node);
            let evidence = ctx.text(node);
            let strs = all_string_arg_texts(node, ctx.source);
            if let Some(path_str) = strs.last() {
                let stem = path_str
                    .rsplit('/')
                    .next()
                    .unwrap_or(path_str)
                    .split('.')
                    .next()
                    .unwrap_or("")
                    .to_owned();
                ctx.push_import(AstImport {
                    path: path_str.clone(),
                    symbols: if stem.is_empty() {
                        Vec::new()
                    } else {
                        vec![stem]
                    },
                    alias: None,
                    is_wildcard: false,
                    is_reexport: false,
                    reexport_visibility: None,
                    kind: "include".to_owned(),
                    line,
                    evidence: clip_str(evidence, 160),
                });
            }
        }
        "class_declaration"
        | "interface_declaration"
        | "trait_declaration"
        | "enum_declaration" => {
            let decl_kind = match kind {
                "interface_declaration" => "interface",
                "trait_declaration" => "trait",
                "enum_declaration" => "enum",
                _ => "class",
            };
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let line = start_line(node);
            let e_line = end_line(node);
            let body_node = node.child_by_field_name("body");
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source);
            let modifiers = collect_php_modifiers(node, ctx.source);

            let mut param_types = Vec::new();
            if let Some(body) = body_node {
                for i in 0..body.child_count() {
                    if let Some(m) = body.child(i)
                        && m.kind() == "method_declaration"
                        && m.child_by_field_name("name")
                            .is_some_and(|n| ctx.text(n) == "__invoke")
                        && let Some(params) = m.child_by_field_name("parameters")
                    {
                        param_types.extend(collect_php_param_type_names(params, ctx.source));
                    }
                }
            }
            emit_php_attributes(ctx, node, Some(&name), &param_types);

            if !name.is_empty() {
                ctx.push_decl(
                    decl_kind,
                    &name,
                    cur_class.clone(),
                    line,
                    e_line,
                    None,
                    modifiers,
                    sig.clone(),
                    sig,
                    String::new(),
                );
                // Inheritance: `base_clause` and `class_interface_clause`
                if let Some(base) = child_by_kind(node, "base_clause") {
                    for p in collect_php_type_list(base, ctx.source) {
                        let inherit_kind = if decl_kind == "interface" {
                            "implements"
                        } else {
                            "extends"
                        };
                        ctx.push_inheritance(&name, &p, inherit_kind, line, ctx.text(base));
                    }
                }
                if let Some(ifaces) = child_by_kind(node, "class_interface_clause") {
                    for p in collect_php_type_list(ifaces, ctx.source) {
                        ctx.push_inheritance(&name, &p, "implements", line, ctx.text(ifaces));
                    }
                }
                class_stack.push((decl_kind.to_owned(), name));
                if let Some(body) = body_node {
                    walk_php(
                        ctx,
                        body,
                        depth + 1,
                        class_stack,
                        fn_depth,
                        this_props,
                        var_types,
                    );
                }
                class_stack.pop();
                if class_stack.is_empty() {
                    this_props.clear();
                }
                return;
            }
        }
        "property_declaration" => {
            if let Some(owner) = cur_class.as_deref() {
                let (vis, mods, _has_vis) = parse_php_member_qualifiers(node, ctx.source);
                let vis_final = vis.or_else(|| Some("public".to_owned()));
                let type_node = node.child_by_field_name("type");
                let type_str = type_node.map(|n| ctx.text(n).trim().to_owned());
                if let Some(ty) = type_node {
                    emit_php_type_refs(ctx, ty, cur_class.as_deref(), "field");
                }
                for prop_el in children_by_kind(node, "property_element") {
                    if let Some(var_node) = child_by_kind(prop_el, "variable_name") {
                        let prop_name = ctx.text(var_node).trim().to_owned();
                        let line = start_line(prop_el);
                        let e_line = end_line(prop_el);
                        let init_node = prop_el.child_by_field_name("default_value");
                        let sig_end = init_node.map_or(prop_el.end_byte(), |i| i.start_byte());
                        let sig = header_without_comments(node, sig_end, ctx.source)
                            .trim_end_matches(['=', ';'])
                            .trim()
                            .to_owned();
                        let params_sig = strip_php_vis_from_sig(&sig);
                        let body_fp = init_node
                            .map(|i| ast_body_fingerprint(i, ctx.source))
                            .unwrap_or_default();

                        if let Some(ref ty) = type_str {
                            this_props.insert(
                                prop_name.trim_start_matches('$').to_owned(),
                                ty.trim_start_matches('?').to_owned(),
                            );
                        }
                        if prop_name == "$table"
                            && let Some(init) = init_node
                            && let Some(tbl) = first_string_arg_text(init, ctx.source)
                                .and_then(|s| clean_sql_table_name(&s))
                        {
                            ctx.push_semantic(
                                "reads_table",
                                format!("table:{tbl}"),
                                Some(owner.to_owned()),
                                line,
                                ctx.text(prop_el),
                            );
                        }
                        ctx.push_decl(
                            "property",
                            &prop_name,
                            Some(owner.to_owned()),
                            line,
                            e_line,
                            vis_final.clone(),
                            mods.clone(),
                            sig,
                            params_sig,
                            body_fp,
                        );
                    }
                }
            }
            return;
        }
        "const_declaration" => {
            if *fn_depth == 0 {
                let (vis, mods, _) = parse_php_member_qualifiers(node, ctx.source);
                let vis_final = vis.or_else(|| Some("public".to_owned()));
                for el in children_by_kind(node, "const_element") {
                    if let Some(name_node) = el.child(0) {
                        let name = ctx.text(name_node).trim().to_owned();
                        let line = start_line(el);
                        let e_line = end_line(el);
                        let val_node = el.child(2);
                        let sig_end = val_node.map_or(el.end_byte(), |v| v.start_byte());
                        let sig = header_without_comments(node, sig_end, ctx.source)
                            .trim_end_matches(['=', ';'])
                            .trim()
                            .to_owned();
                        let params_sig = strip_php_vis_from_sig(&sig);
                        let body_fp = val_node
                            .map(|v| ast_body_fingerprint(v, ctx.source))
                            .unwrap_or_default();
                        ctx.push_decl(
                            "const",
                            &name,
                            cur_class.clone(),
                            line,
                            e_line,
                            vis_final.clone(),
                            mods.clone(),
                            sig,
                            params_sig,
                            body_fp,
                        );
                    }
                }
            }
            return;
        }
        "method_declaration" | "function_definition" => {
            let is_method = kind == "method_declaration";
            if !is_method && *fn_depth > 0 {
                return;
            }
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let line = start_line(node);
            let e_line = end_line(node);
            let body_node = node.child_by_field_name("body");
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source)
                .trim_end_matches(';')
                .trim()
                .to_owned();
            let params_sig = strip_php_vis_from_sig(&sig);
            let body_fp = body_node
                .map(|b| ast_body_fingerprint(b, ctx.source))
                .unwrap_or_default();
            let (vis, mods, _) = parse_php_member_qualifiers(node, ctx.source);
            let vis_final = vis.or_else(|| Some("public".to_owned()));

            let params_node = node.child_by_field_name("parameters");
            let param_types = params_node
                .map(|p| collect_php_param_type_names(p, ctx.source))
                .unwrap_or_default();
            let sym_label = if let Some(ref owner) = cur_class {
                format!("{owner}::{name}")
            } else {
                name.clone()
            };
            emit_php_attributes(ctx, node, Some(&sym_label), &param_types);

            if !name.is_empty() {
                ctx.push_decl(
                    if is_method { "method" } else { "function" },
                    &name,
                    if is_method {
                        cur_class.clone()
                    } else {
                        Some(ctx.rel_path.clone())
                    },
                    line,
                    e_line,
                    vis_final,
                    mods,
                    sig,
                    params_sig,
                    body_fp,
                );
            }

            let mut local_vars = var_types.clone();
            if let Some(params) = params_node {
                extract_php_params(
                    ctx,
                    params,
                    cur_class.as_deref(),
                    &name,
                    this_props,
                    &mut local_vars,
                );
            }
            if let Some(ret) = node.child_by_field_name("return_type") {
                emit_php_type_refs(ctx, ret, Some(&sym_label), "return_type");
            }
            if let Some(body) = body_node {
                *fn_depth += 1;
                walk_php(
                    ctx,
                    body,
                    depth + 1,
                    class_stack,
                    fn_depth,
                    this_props,
                    &mut local_vars,
                );
                *fn_depth -= 1;
            }
            return;
        }
        "assignment_expression" => {
            if let (Some(lhs), Some(rhs)) = (
                node.child_by_field_name("left"),
                node.child_by_field_name("right"),
            ) {
                if let Some(ty) = infer_php_expr_type(ctx, rhs, this_props, var_types) {
                    let lhs_txt = ctx.text(lhs).trim();
                    if let Some(prop) = lhs_txt.strip_prefix("$this->") {
                        this_props.insert(prop.to_owned(), ty);
                    } else if let Some(var) = lhs_txt.strip_prefix('$')
                        && var.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    {
                        var_types.insert(var.to_owned(), ty);
                    }
                }
                if lhs.kind() == "member_access_expression"
                    || lhs.kind() == "nullsafe_member_access_expression"
                    || lhs.kind() == "scoped_property_access_expression"
                {
                    emit_php_member_access(ctx, lhs, true, this_props, var_types);
                    walk_php(
                        ctx,
                        rhs,
                        depth + 1,
                        class_stack,
                        fn_depth,
                        this_props,
                        var_types,
                    );
                    return;
                }
            }
        }
        "augmented_assignment_expression" => {
            if let Some(lhs) = node.child_by_field_name("left")
                && matches!(
                    lhs.kind(),
                    "member_access_expression"
                        | "nullsafe_member_access_expression"
                        | "scoped_property_access_expression"
                )
            {
                emit_php_member_access(ctx, lhs, true, this_props, var_types);
                if let Some(rhs) = node.child_by_field_name("right") {
                    walk_php(
                        ctx,
                        rhs,
                        depth + 1,
                        class_stack,
                        fn_depth,
                        this_props,
                        var_types,
                    );
                }
                return;
            }
        }
        "object_creation_expression" => {
            let line = start_line(node);
            let evidence = ctx.text(node);
            for i in 0..node.child_count() {
                if let Some(ch) = node.child(i)
                    && matches!(ch.kind(), "name" | "qualified_name")
                {
                    let cls = ctx.text(ch).trim().to_owned();
                    ctx.push_ref(AstRef {
                        target_name: cls,
                        member: None,
                        context: "constructor".to_owned(),
                        access: "call".to_owned(),
                        member_kind: None,
                        owner_type: None,
                        enclosing_symbol: cur_class.clone(),
                        is_static: false,
                        line,
                        evidence: clip_str(evidence, 160),
                    });
                    break;
                }
            }
        }
        "scoped_call_expression" => {
            if let (Some(scope), Some(name_node)) = (
                node.child_by_field_name("scope"),
                node.child_by_field_name("name"),
            ) {
                let scope_txt = ctx.text(scope).trim().to_owned();
                let mem_txt = ctx.text(name_node).trim().to_owned();
                let line = start_line(node);
                let evidence = ctx.text(node);
                let args_node = node.child_by_field_name("arguments");
                let args_txt = args_node.map(|a| ctx.text(a)).unwrap_or("");
                let str_args = args_node
                    .map(|a| all_string_arg_texts(a, ctx.source))
                    .unwrap_or_default();
                let chain_txt = node.parent().map_or(evidence, |p| ctx.text(p));
                emit_semantic_from_call(
                    ctx,
                    Some(&scope_txt),
                    &mem_txt,
                    args_txt,
                    &str_args,
                    chain_txt,
                    cur_class.clone(),
                    line,
                );
                let short_scope = scope_txt
                    .rsplit('\\')
                    .next()
                    .unwrap_or(&scope_txt)
                    .to_owned();
                ctx.push_ref(AstRef {
                    target_name: scope_txt,
                    member: Some(mem_txt),
                    context: "call".to_owned(),
                    access: "call".to_owned(),
                    member_kind: Some("method".to_owned()),
                    owner_type: Some(short_scope),
                    enclosing_symbol: cur_class.clone(),
                    is_static: true,
                    line,
                    evidence: clip_str(evidence, 160),
                });
            }
        }
        "member_call_expression" | "nullsafe_member_call_expression" => {
            if let (Some(obj), Some(name_node)) = (
                node.child_by_field_name("object"),
                node.child_by_field_name("name"),
            ) {
                let obj_txt = ctx.text(obj).trim();
                let mem_txt = ctx.text(name_node).trim().to_owned();
                let line = start_line(node);
                let evidence = ctx.text(node);
                let args_node = node.child_by_field_name("arguments");
                let args_txt = args_node.map(|a| ctx.text(a)).unwrap_or("");
                let str_args = args_node
                    .map(|a| all_string_arg_texts(a, ctx.source))
                    .unwrap_or_default();
                emit_semantic_from_call(
                    ctx,
                    Some(obj_txt),
                    &mem_txt,
                    args_txt,
                    &str_args,
                    evidence,
                    cur_class.clone(),
                    line,
                );
                if let Some(owner_ty) = resolve_php_receiver_type(obj_txt, this_props, var_types) {
                    let short = owner_ty.rsplit('\\').next().unwrap_or(&owner_ty).to_owned();
                    ctx.push_ref(AstRef {
                        target_name: owner_ty,
                        member: Some(mem_txt),
                        context: "call".to_owned(),
                        access: "call".to_owned(),
                        member_kind: Some("method".to_owned()),
                        owner_type: Some(short),
                        enclosing_symbol: cur_class.clone(),
                        is_static: false,
                        line,
                        evidence: clip_str(evidence, 160),
                    });
                }
            }
        }
        "function_call_expression" => {
            if let Some(func_node) = node.child_by_field_name("function") {
                let fn_name = ctx.text(func_node).trim().to_owned();
                let line = start_line(node);
                let evidence = ctx.text(node);
                let args_node = node.child_by_field_name("arguments");
                let args_txt = args_node.map(|a| ctx.text(a)).unwrap_or("");
                let str_args = args_node
                    .map(|a| all_string_arg_texts(a, ctx.source))
                    .unwrap_or_default();
                emit_semantic_from_call(
                    ctx,
                    None,
                    &fn_name,
                    args_txt,
                    &str_args,
                    evidence,
                    cur_class.clone(),
                    line,
                );
                ctx.push_ref(AstRef {
                    target_name: fn_name,
                    member: None,
                    context: "call".to_owned(),
                    access: "call".to_owned(),
                    member_kind: Some("function".to_owned()),
                    owner_type: None,
                    enclosing_symbol: cur_class.clone(),
                    is_static: false,
                    line,
                    evidence: clip_str(evidence, 160),
                });
            }
        }
        "member_access_expression"
        | "nullsafe_member_access_expression"
        | "scoped_property_access_expression"
        | "class_constant_access_expression" => {
            emit_php_member_access(ctx, node, false, this_props, var_types);
        }
        "binary_expression" => {
            let txt = ctx.text(node);
            if txt.contains("instanceof")
                && let Some(right) = node.child_by_field_name("right")
                && matches!(right.kind(), "name" | "qualified_name")
            {
                let cls = ctx.text(right).trim().to_owned();
                ctx.push_ref(AstRef {
                    target_name: cls,
                    member: None,
                    context: "parameter_type".to_owned(),
                    access: "type".to_owned(),
                    member_kind: None,
                    owner_type: None,
                    enclosing_symbol: cur_class.clone(),
                    is_static: false,
                    line: start_line(node),
                    evidence: clip_str(txt, 160),
                });
            }
        }
        "catch_clause" => {
            if let Some(ty) = node.child_by_field_name("type") {
                emit_php_type_refs(ctx, ty, cur_class.as_deref(), "parameter_type");
            }
        }
        _ => {}
    }

    for i in 0..node.child_count() {
        if let Some(ch) = node.child(i) {
            walk_php(
                ctx,
                ch,
                depth + 1,
                class_stack,
                fn_depth,
                this_props,
                var_types,
            );
        }
    }
}

fn extract_php_namespace_use(ctx: &mut ExtractCtx<'_>, node: Node<'_>) {
    let line = start_line(node);
    let evidence = ctx.text(node);
    let mut prefix: Option<String> = None;

    for i in 0..node.child_count() {
        let Some(ch) = node.child(i) else {
            continue;
        };
        match ch.kind() {
            "namespace_name" => {
                prefix = Some(ctx.text(ch).trim().trim_matches('\\').to_owned());
            }
            "namespace_use_clause" => {
                emit_php_use_clause(ctx, ch, prefix.as_deref(), line, evidence);
            }
            "namespace_use_group" => {
                for clause in children_by_kind(ch, "namespace_use_clause") {
                    emit_php_use_clause(ctx, clause, prefix.as_deref(), line, evidence);
                }
            }
            _ => {}
        }
    }
}

fn emit_php_use_clause(
    ctx: &mut ExtractCtx<'_>,
    clause: Node<'_>,
    prefix: Option<&str>,
    line: u32,
    evidence: &str,
) {
    let mut raw_path = String::new();
    for i in 0..clause.child_count() {
        if let Some(ch) = clause.child(i)
            && matches!(ch.kind(), "name" | "qualified_name" | "namespace_name")
        {
            raw_path = ctx.text(ch).trim().trim_matches('\\').to_owned();
            break;
        }
    }
    if raw_path.is_empty() {
        return;
    }
    let full = match prefix {
        Some(p) if !p.is_empty() => format!("{p}\\{raw_path}"),
        _ => raw_path,
    };
    let alias = clause
        .child_by_field_name("alias")
        .map(|a| ctx.text(a).trim().to_owned());
    let short = full.rsplit('\\').next().unwrap_or(&full).to_owned();
    ctx.push_import(AstImport {
        path: full,
        symbols: vec![short],
        alias,
        is_wildcard: false,
        is_reexport: false,
        reexport_visibility: None,
        kind: "import".to_owned(),
        line,
        evidence: clip_str(evidence, 160),
    });
}

fn collect_php_modifiers(node: Node<'_>, source: &str) -> Vec<String> {
    let mut mods = Vec::new();
    for i in 0..node.child_count() {
        if let Some(ch) = node.child(i)
            && matches!(
                ch.kind(),
                "abstract_modifier" | "final_modifier" | "readonly_modifier" | "static_modifier"
            )
        {
            push_unique_mod(&mut mods, node_text(ch, source).trim());
        }
    }
    mods
}

fn parse_php_member_qualifiers(
    node: Node<'_>,
    source: &str,
) -> (Option<String>, Vec<String>, bool) {
    let mut vis_tokens: Vec<String> = Vec::new();
    let mut mods: Vec<String> = Vec::new();
    let mut has_vis = false;
    for i in 0..node.child_count() {
        let Some(ch) = node.child(i) else {
            continue;
        };
        match ch.kind() {
            "visibility_modifier" => {
                has_vis = true;
                let txt = node_text(ch, source)
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                vis_tokens.push(txt);
            }
            "abstract_modifier" | "final_modifier" | "readonly_modifier" | "static_modifier" => {
                push_unique_mod(&mut mods, node_text(ch, source).trim());
            }
            _ => {}
        }
    }
    let vis = if vis_tokens.is_empty() {
        None
    } else if let Some(set_vis) = vis_tokens.iter().find(|v| v.contains("(set)")) {
        Some(set_vis.clone())
    } else {
        Some(vis_tokens[0].clone())
    };
    (vis, mods, has_vis)
}

fn strip_php_vis_from_sig(sig: &str) -> String {
    let mut cur = sig.trim();
    let mut kept = Vec::new();
    loop {
        cur = cur.trim_start();
        let next = cur.split_whitespace().next().unwrap_or("");
        if matches!(next, "public" | "protected" | "private" | "var")
            || next.starts_with("public(")
            || next.starts_with("protected(")
            || next.starts_with("private(")
        {
            cur = cur[next.len()..].trim_start();
            if cur.starts_with("(set)") {
                cur = cur[5..].trim_start();
            }
            continue;
        }
        if matches!(next, "abstract" | "final" | "readonly" | "static") {
            kept.push(next);
            cur = cur[next.len()..].trim_start();
            continue;
        }
        break;
    }
    if kept.is_empty() {
        cur.to_owned()
    } else if cur.is_empty() {
        kept.join(" ")
    } else {
        format!("{} {cur}", kept.join(" "))
    }
}

fn collect_php_type_list(clause: Node<'_>, source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for i in 0..clause.child_count() {
        if let Some(ch) = clause.child(i)
            && matches!(ch.kind(), "name" | "qualified_name")
        {
            out.push(node_text(ch, source).trim().to_owned());
        }
    }
    out
}

fn collect_php_param_type_names(params_node: Node<'_>, source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for i in 0..params_node.child_count() {
        if let Some(ch) = params_node.child(i)
            && let Some(ty) = ch.child_by_field_name("type")
        {
            let txt = node_text(ty, source).trim().trim_start_matches('?');
            let short = txt.rsplit('\\').next().unwrap_or(txt);
            if !short.is_empty() {
                out.push(short.to_owned());
            }
        }
    }
    out
}

fn emit_php_attributes(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    annotated_symbol: Option<&str>,
    param_types: &[String],
) {
    let Some(attr_list) = node.child_by_field_name("attributes") else {
        return;
    };
    let mut stack = vec![attr_list];
    while let Some(cur) = stack.pop() {
        if cur.kind() == "attribute" {
            let attr_text = ctx.text(cur);
            let attr_name = cur.child(0).map(|n| ctx.text(n)).unwrap_or(attr_text);
            let strs = cur
                .child_by_field_name("parameters")
                .map(|p| all_string_arg_texts(p, ctx.source))
                .unwrap_or_default();
            emit_semantic_from_attribute(
                ctx,
                attr_name,
                attr_text,
                &strs,
                annotated_symbol,
                param_types,
                start_line(cur),
            );
            continue;
        }
        for i in (0..cur.child_count()).rev() {
            if let Some(ch) = cur.child(i) {
                stack.push(ch);
            }
        }
    }
}

fn emit_php_type_refs(
    ctx: &mut ExtractCtx<'_>,
    ty_node: Node<'_>,
    enclosing: Option<&str>,
    context: &str,
) {
    let line = start_line(ty_node);
    let evidence = ctx.text(ty_node);
    let mut stack = vec![ty_node];
    while let Some(cur) = stack.pop() {
        if matches!(cur.kind(), "name" | "qualified_name" | "named_type") && cur.child_count() == 0
        {
            let txt = ctx.text(cur).trim().trim_start_matches('?');
            if !txt.is_empty() {
                ctx.push_ref(AstRef {
                    target_name: txt.to_owned(),
                    member: None,
                    context: context.to_owned(),
                    access: "type".to_owned(),
                    member_kind: None,
                    owner_type: None,
                    enclosing_symbol: enclosing.map(str::to_owned),
                    is_static: false,
                    line,
                    evidence: clip_str(evidence, 160),
                });
            }
            continue;
        }
        if cur.kind() == "qualified_name" {
            let txt = ctx.text(cur).trim().to_owned();
            if !txt.is_empty() {
                ctx.push_ref(AstRef {
                    target_name: txt,
                    member: None,
                    context: context.to_owned(),
                    access: "type".to_owned(),
                    member_kind: None,
                    owner_type: None,
                    enclosing_symbol: enclosing.map(str::to_owned),
                    is_static: false,
                    line,
                    evidence: clip_str(evidence, 160),
                });
            }
            continue;
        }
        for i in (0..cur.child_count()).rev() {
            if let Some(ch) = cur.child(i) {
                stack.push(ch);
            }
        }
    }
}

fn extract_php_params(
    ctx: &mut ExtractCtx<'_>,
    params_node: Node<'_>,
    owner_class: Option<&str>,
    fn_name: &str,
    this_props: &mut BTreeMap<String, String>,
    var_types: &mut BTreeMap<String, String>,
) {
    let is_ctor = fn_name == "__construct";
    for i in 0..params_node.child_count() {
        let Some(p) = params_node.child(i) else {
            continue;
        };
        if !matches!(
            p.kind(),
            "simple_parameter" | "property_promotion_parameter"
        ) {
            continue;
        }
        let ty_node = p.child_by_field_name("type");
        let ty_str = ty_node.map(|t| {
            ctx.text(t)
                .split('|')
                .map(str::trim)
                .find(|part| !part.eq_ignore_ascii_case("null") && !part.is_empty())
                .unwrap_or("")
                .trim_start_matches('?')
                .to_owned()
        });
        let var_name = p
            .child_by_field_name("name")
            .map(|n| ctx.text(n).trim().to_owned())
            .unwrap_or_default();
        if let Some(ty) = ty_node {
            emit_php_type_refs(ctx, ty, owner_class, "parameter_type");
        }
        if let Some(ref ty) = ty_str
            && !ty.is_empty()
            && !var_name.is_empty()
        {
            var_types.insert(var_name.trim_start_matches('$').to_owned(), ty.clone());
        }
        if p.kind() == "property_promotion_parameter"
            && is_ctor
            && let Some(owner) = owner_class
        {
            let (vis, mods, has_vis) = parse_php_member_qualifiers(p, ctx.source);
            let has_readonly =
                p.child_by_field_name("readonly").is_some() || mods.iter().any(|m| m == "readonly");
            if has_vis || has_readonly {
                let mut all_mods = mods;
                if has_readonly {
                    push_unique_mod(&mut all_mods, "readonly");
                }
                let line = start_line(p);
                let e_line = end_line(p);
                let def_val = p.child_by_field_name("default_value");
                let sig_end = def_val.map_or(p.end_byte(), |d| d.start_byte());
                let sig = header_without_comments(p, sig_end, ctx.source)
                    .trim_end_matches('=')
                    .trim()
                    .to_owned();
                let params_sig = strip_php_vis_from_sig(&sig);
                let body_fp = def_val
                    .map(|d| ast_body_fingerprint(d, ctx.source))
                    .unwrap_or_default();
                if let Some(ref ty) = ty_str
                    && !ty.is_empty()
                {
                    this_props.insert(var_name.trim_start_matches('$').to_owned(), ty.clone());
                }
                ctx.push_decl(
                    "property",
                    &var_name,
                    Some(owner.to_owned()),
                    line,
                    e_line,
                    vis,
                    all_mods,
                    sig,
                    params_sig,
                    body_fp,
                );
            }
        }
    }
}

fn infer_php_expr_type(
    ctx: &ExtractCtx<'_>,
    rhs: Node<'_>,
    this_props: &BTreeMap<String, String>,
    var_types: &BTreeMap<String, String>,
) -> Option<String> {
    match rhs.kind() {
        "object_creation_expression" => {
            for i in 0..rhs.child_count() {
                if let Some(ch) = rhs.child(i)
                    && matches!(ch.kind(), "name" | "qualified_name")
                {
                    return Some(ctx.text(ch).trim().to_owned());
                }
            }
            None
        }
        "variable_name" => {
            let v = ctx.text(rhs).trim().trim_start_matches('$');
            var_types.get(v).cloned()
        }
        "member_access_expression" | "nullsafe_member_access_expression" => {
            let txt = ctx.text(rhs).trim();
            if let Some(prop) = txt.strip_prefix("$this->") {
                return this_props.get(prop).cloned();
            }
            None
        }
        _ => None,
    }
}

fn resolve_php_receiver_type(
    recv: &str,
    this_props: &BTreeMap<String, String>,
    var_types: &BTreeMap<String, String>,
) -> Option<String> {
    let trimmed = recv.trim();
    if let Some(prop) = trimmed
        .strip_prefix("$this->")
        .or_else(|| trimmed.strip_prefix("$this?->"))
    {
        return this_props.get(prop.trim()).cloned();
    }
    if let Some(var) = trimmed.strip_prefix('$') {
        return var_types.get(var).cloned();
    }
    None
}

fn emit_php_member_access(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    is_write: bool,
    this_props: &BTreeMap<String, String>,
    var_types: &BTreeMap<String, String>,
) {
    let line = start_line(node);
    let evidence = ctx.text(node);
    match node.kind() {
        "scoped_property_access_expression" => {
            if let (Some(scope), Some(name_node)) = (
                node.child_by_field_name("scope"),
                node.child_by_field_name("name"),
            ) {
                let scope_txt = ctx.text(scope).trim().to_owned();
                let mem_txt = ctx.text(name_node).trim().to_owned();
                let short_scope = scope_txt
                    .rsplit('\\')
                    .next()
                    .unwrap_or(&scope_txt)
                    .to_owned();
                ctx.push_ref(AstRef {
                    target_name: scope_txt,
                    member: Some(mem_txt),
                    context: "field".to_owned(),
                    access: if is_write { "write" } else { "static_read" }.to_owned(),
                    member_kind: Some("property".to_owned()),
                    owner_type: Some(short_scope),
                    enclosing_symbol: None,
                    is_static: true,
                    line,
                    evidence: clip_str(evidence, 160),
                });
            }
        }
        "class_constant_access_expression" => {
            let scope_txt = node.child(0).map(|n| ctx.text(n).trim()).unwrap_or("");
            let const_txt = node.child(2).map(|n| ctx.text(n).trim()).unwrap_or("");
            if !scope_txt.is_empty() && !const_txt.is_empty() {
                if const_txt == "class" {
                    ctx.push_ref(AstRef {
                        target_name: scope_txt.to_owned(),
                        member: None,
                        context: "parameter_type".to_owned(),
                        access: "type".to_owned(),
                        member_kind: None,
                        owner_type: None,
                        enclosing_symbol: None,
                        is_static: true,
                        line,
                        evidence: clip_str(evidence, 160),
                    });
                } else {
                    let short_scope = scope_txt
                        .rsplit('\\')
                        .next()
                        .unwrap_or(scope_txt)
                        .to_owned();
                    ctx.push_ref(AstRef {
                        target_name: scope_txt.to_owned(),
                        member: Some(const_txt.to_owned()),
                        context: "field".to_owned(),
                        access: "const".to_owned(),
                        member_kind: Some("const".to_owned()),
                        owner_type: Some(short_scope),
                        enclosing_symbol: None,
                        is_static: true,
                        line,
                        evidence: clip_str(evidence, 160),
                    });
                }
            }
        }
        "member_access_expression" | "nullsafe_member_access_expression" => {
            if let (Some(obj), Some(name_node)) = (
                node.child_by_field_name("object"),
                node.child_by_field_name("name"),
            ) {
                let obj_txt = ctx.text(obj).trim();
                let mem_raw = ctx.text(name_node).trim();
                let mem_name = if mem_raw.starts_with('$') {
                    mem_raw.to_owned()
                } else {
                    format!("${mem_raw}")
                };
                if let Some(owner_ty) = resolve_php_receiver_type(obj_txt, this_props, var_types) {
                    let short = owner_ty.rsplit('\\').next().unwrap_or(&owner_ty).to_owned();
                    ctx.push_ref(AstRef {
                        target_name: owner_ty,
                        member: Some(mem_name),
                        context: "field".to_owned(),
                        access: if is_write { "write" } else { "read" }.to_owned(),
                        member_kind: Some("property".to_owned()),
                        owner_type: Some(short),
                        enclosing_symbol: None,
                        is_static: false,
                        line,
                        evidence: clip_str(evidence, 160),
                    });
                }
            }
        }
        _ => {}
    }
}

// =========================================================================
// 2. Rust Extraction
// =========================================================================
