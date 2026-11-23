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
            let mut fallback_param_types = Vec::new();
            if let Some(body) = body_node {
                for i in 0..body.child_count() {
                    if let Some(m) = body.child(i)
                        && m.kind() == "method_declaration"
                        && let Some(params) = m.child_by_field_name("parameters")
                    {
                        let m_name = m
                            .child_by_field_name("name")
                            .map(|n| ctx.text(n).trim())
                            .unwrap_or("");
                        let pt = collect_php_param_type_names(params, ctx.source);
                        if matches!(m_name, "__invoke" | "handle") {
                            param_types.extend(pt);
                        } else if fallback_param_types.is_empty() && !pt.is_empty() {
                            fallback_param_types.extend(pt);
                        }
                    }
                }
            }
            if param_types.is_empty() {
                param_types = fallback_param_types;
            }
            let class_route_prefix = if decl_kind == "class" {
                extract_php_class_route_prefix(node, ctx.source)
            } else {
                None
            };
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
                // Inheritance: `base_clause` (`extends` for classes & interfaces) and `class_interface_clause`
                if let Some(base) = child_by_kind(node, "base_clause") {
                    for p in collect_php_type_list(base, ctx.source) {
                        ctx.push_inheritance(&name, &p, "extends", line, ctx.text(base));
                    }
                }
                if let Some(ifaces) = child_by_kind(node, "class_interface_clause") {
                    for p in collect_php_type_list(ifaces, ctx.source) {
                        ctx.push_inheritance(&name, &p, "implements", line, ctx.text(ifaces));
                    }
                }
                let attr_prefix_mark = class_route_prefix
                    .as_deref()
                    .map(|p| ctx.push_attr_route_prefix(p));
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
                if let Some(mark) = attr_prefix_mark {
                    ctx.truncate_attr_route_prefixes(mark);
                }
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
            let params = params_node
                .map(|p| collect_php_params(p, ctx.source))
                .unwrap_or_default();
            let param_types: Vec<String> = params.iter().map(|(ty, _)| ty.clone()).collect();
            let sym_label = if let Some(ref owner) = cur_class {
                format!("{owner}::{name}")
            } else {
                name.clone()
            };
            emit_php_attributes(ctx, node, Some(&sym_label), &param_types);
            if matches!(name.as_str(), "handle" | "__invoke")
                && let Some((first_ty, first_param)) = params.first()
            {
                let short = first_ty
                    .rsplit(['\\', ':', '.'])
                    .next()
                    .unwrap_or(first_ty)
                    .trim();
                let is_builtin = matches!(
                    short.to_ascii_lowercase().as_str(),
                    "int"
                        | "float"
                        | "string"
                        | "bool"
                        | "array"
                        | "object"
                        | "callable"
                        | "iterable"
                        | "mixed"
                        | "void"
                        | "never"
                        | "null"
                        | "false"
                        | "true"
                        | "self"
                        | "static"
                        | "parent"
                );
                let is_event_type = short.ends_with("Event")
                    || short.ends_with("Message")
                    || short.ends_with("Job")
                    || short.ends_with("Notification");
                let is_event_param = matches!(
                    first_param.as_str(),
                    "$event" | "$msg" | "$message" | "$job" | "$notification"
                );
                if !is_builtin && (is_event_type || is_event_param) {
                    ctx.push_semantic(
                        "handles_event",
                        format!("event:{short}"),
                        Some(sym_label.clone()),
                        line,
                        &sig,
                    );
                }
            }

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
                    emit_php_member_access(
                        ctx,
                        lhs,
                        true,
                        cur_class.as_deref(),
                        this_props,
                        var_types,
                    );
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
                emit_php_member_access(ctx, lhs, true, cur_class.as_deref(), this_props, var_types);
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
                let group_prefixes =
                    extract_php_route_group_prefixes(Some(scope), &mem_txt, args_node, ctx.source);
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
                if scope_txt.eq_ignore_ascii_case("View")
                    && mem_txt.eq_ignore_ascii_case("make")
                    && let Some(view_name) = str_args.first()
                {
                    push_blade_view_import(ctx, view_name, line, evidence);
                }
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
                if !group_prefixes.is_empty()
                    && let Some(args) = args_node
                {
                    let mark = ctx.push_call_route_prefix("");
                    for pref in &group_prefixes {
                        let _ = ctx.push_call_route_prefix(pref);
                    }
                    walk_php(
                        ctx,
                        args,
                        depth + 1,
                        class_stack,
                        fn_depth,
                        this_props,
                        var_types,
                    );
                    ctx.truncate_call_route_prefixes(mark);
                    return;
                }
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
                let group_prefixes =
                    extract_php_route_group_prefixes(Some(obj), &mem_txt, args_node, ctx.source);
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
                if let Some(owner_ty) = resolve_php_receiver_type_with_self(
                    obj_txt,
                    cur_class.as_deref(),
                    this_props,
                    var_types,
                ) {
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
                if !group_prefixes.is_empty()
                    && let Some(args) = args_node
                {
                    walk_php(
                        ctx,
                        obj,
                        depth + 1,
                        class_stack,
                        fn_depth,
                        this_props,
                        var_types,
                    );
                    let mark = ctx.push_call_route_prefix("");
                    for pref in &group_prefixes {
                        let _ = ctx.push_call_route_prefix(pref);
                    }
                    walk_php(
                        ctx,
                        args,
                        depth + 1,
                        class_stack,
                        fn_depth,
                        this_props,
                        var_types,
                    );
                    ctx.truncate_call_route_prefixes(mark);
                    return;
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
                if fn_name
                    .trim_start_matches('\\')
                    .eq_ignore_ascii_case("view")
                    && let Some(view_name) = str_args.first()
                {
                    push_blade_view_import(ctx, view_name, line, evidence);
                }
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
            emit_php_member_access(
                ctx,
                node,
                false,
                cur_class.as_deref(),
                this_props,
                var_types,
            );
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

fn collect_php_params(params_node: Node<'_>, source: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for i in 0..params_node.child_count() {
        if let Some(ch) = params_node.child(i)
            && let Some(ty) = ch.child_by_field_name("type")
        {
            let txt = node_text(ty, source).trim().trim_start_matches('?');
            let short = txt.rsplit('\\').next().unwrap_or(txt).trim();
            if !short.is_empty() {
                let param_name = ch
                    .child_by_field_name("name")
                    .map(|n| node_text(n, source).trim().to_owned())
                    .unwrap_or_default();
                out.push((short.to_owned(), param_name));
            }
        }
    }
    out
}

fn collect_php_param_type_names(params_node: Node<'_>, source: &str) -> Vec<String> {
    collect_php_params(params_node, source)
        .into_iter()
        .map(|(ty, _)| ty)
        .collect()
}

fn extract_php_route_attr_path(params: Node<'_>, source: &str) -> Option<String> {
    let mut positional: Option<String> = None;
    let mut saw_argument = false;
    let mut stack = vec![params];
    while let Some(cur) = stack.pop() {
        if cur.kind() == "argument" {
            saw_argument = true;
            let name_node = cur
                .child_by_field_name("name")
                .or_else(|| cur.named_child(0).filter(|c| c.kind() == "name"));
            if let Some(name_n) = name_node {
                let arg_name = node_text(name_n, source).trim().to_ascii_lowercase();
                if matches!(
                    arg_name.as_str(),
                    "path" | "prefix" | "value" | "uri" | "url"
                ) && let Some(val) = all_string_arg_texts(cur, source)
                    .into_iter()
                    .map(|s| s.trim().to_owned())
                    .find(|s| !s.is_empty())
                {
                    return Some(val);
                }
            } else if positional.is_none() {
                let expr = cur.named_child(0).unwrap_or(cur);
                if is_string_kind(expr.kind())
                    && let Some(val) = unquote_literal(node_text(expr, source))
                {
                    let trimmed = val.trim();
                    if !trimmed.is_empty() {
                        positional = Some(trimmed.to_owned());
                    }
                }
            }
            continue;
        }
        for i in (0..cur.child_count()).rev() {
            if let Some(ch) = cur.child(i) {
                stack.push(ch);
            }
        }
    }
    if positional.is_some() || saw_argument {
        positional
    } else {
        all_string_arg_texts(params, source)
            .into_iter()
            .map(|s| s.trim().to_owned())
            .find(|s| !s.is_empty())
    }
}

fn is_php_route_attr_short(short_lower: &str) -> bool {
    matches!(
        short_lower,
        "route"
            | "requestmapping"
            | "path"
            | "getmapping"
            | "httpget"
            | "get"
            | "postmapping"
            | "httppost"
            | "post"
            | "putmapping"
            | "httpput"
            | "put"
            | "deletemapping"
            | "httpdelete"
            | "delete"
            | "patchmapping"
            | "httppatch"
            | "patch"
    )
}

fn extract_php_class_route_prefix(node: Node<'_>, source: &str) -> Option<String> {
    let attr_list = node.child_by_field_name("attributes")?;
    let mut stack = vec![attr_list];
    while let Some(cur) = stack.pop() {
        if cur.kind() == "attribute" {
            let attr_text = node_text(cur, source);
            let attr_name = cur
                .child(0)
                .map(|n| node_text(n, source))
                .unwrap_or(attr_text);
            let short = attr_name
                .rsplit(['\\', ':', '.'])
                .next()
                .unwrap_or(attr_name)
                .trim_start_matches(['@', '#'])
                .to_ascii_lowercase();
            if matches!(
                short.as_str(),
                "route" | "requestmapping" | "path" | "controller"
            ) && let Some(params) = cur.child_by_field_name("parameters")
                && let Some(path) = extract_php_route_attr_path(params, source)
            {
                return Some(path);
            }
            continue;
        }
        for i in (0..cur.child_count()).rev() {
            if let Some(ch) = cur.child(i) {
                stack.push(ch);
            }
        }
    }
    None
}

fn extract_php_route_group_prefixes(
    receiver: Option<Node<'_>>,
    callee: &str,
    args_node: Option<Node<'_>>,
    source: &str,
) -> Vec<String> {
    if !callee.eq_ignore_ascii_case("group") {
        return Vec::new();
    }
    let mut chain_prefixes = Vec::new();
    let mut looks_like_route_chain = false;
    let mut cursor = receiver;
    while let Some(cur) = cursor {
        match cur.kind() {
            "member_call_expression" | "nullsafe_member_call_expression" => {
                if let Some(name_n) = cur.child_by_field_name("name") {
                    let m = node_text(name_n, source).trim();
                    if m.eq_ignore_ascii_case("prefix")
                        && let Some(a) = cur.child_by_field_name("arguments")
                        && let Some(p) = first_string_arg_text(a, source)
                    {
                        chain_prefixes.push(p);
                        looks_like_route_chain = true;
                    }
                }
                cursor = cur.child_by_field_name("object");
            }
            "scoped_call_expression" => {
                if let (Some(sc), Some(name_n)) = (
                    cur.child_by_field_name("scope"),
                    cur.child_by_field_name("name"),
                ) {
                    let sc_low = node_text(sc, source).trim().to_ascii_lowercase();
                    let m = node_text(name_n, source).trim();
                    if sc_low.contains("route") {
                        looks_like_route_chain = true;
                    }
                    if m.eq_ignore_ascii_case("prefix")
                        && let Some(a) = cur.child_by_field_name("arguments")
                        && let Some(p) = first_string_arg_text(a, source)
                    {
                        chain_prefixes.push(p);
                        looks_like_route_chain = true;
                    }
                }
                break;
            }
            _ => {
                let txt = node_text(cur, source).trim().to_ascii_lowercase();
                if txt.contains("route") || txt == "$this" || txt == "$app" {
                    looks_like_route_chain = true;
                }
                break;
            }
        }
    }
    chain_prefixes.reverse();

    if looks_like_route_chain && let Some(args) = args_node {
        for i in 0..args.named_child_count() {
            if let Some(arg) = args.named_child(i) {
                let expr = if arg.kind() == "argument" {
                    arg.named_child(0).unwrap_or(arg)
                } else {
                    arg
                };
                if expr.kind() == "array_creation_expression" {
                    for el in children_by_kind(expr, "array_element_initializer") {
                        let txt = node_text(el, source);
                        if let Some((k, v)) = txt.split_once("=>")
                            && unquote_literal(k.trim())
                                .is_some_and(|key| key.eq_ignore_ascii_case("prefix"))
                            && let Some(p) = unquote_literal(v.trim())
                        {
                            chain_prefixes.push(p);
                        }
                    }
                }
            }
        }
    }
    if looks_like_route_chain {
        chain_prefixes
    } else {
        Vec::new()
    }
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
            let short_lower = attr_name
                .rsplit(['\\', ':', '.'])
                .next()
                .unwrap_or(attr_name)
                .trim_start_matches(['@', '#'])
                .to_ascii_lowercase();
            let strs = cur
                .child_by_field_name("parameters")
                .map(|p| {
                    if is_php_route_attr_short(&short_lower) {
                        extract_php_route_attr_path(p, ctx.source)
                            .into_iter()
                            .collect()
                    } else {
                        all_string_arg_texts(p, ctx.source)
                    }
                })
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

fn resolve_php_receiver_type_with_self(
    recv: &str,
    cur_class: Option<&str>,
    this_props: &BTreeMap<String, String>,
    var_types: &BTreeMap<String, String>,
) -> Option<String> {
    let trimmed = recv.trim();
    if trimmed == "$this" {
        return cur_class.map(str::to_owned);
    }
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
    cur_class: Option<&str>,
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
                    enclosing_symbol: cur_class.map(str::to_owned),
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
                        enclosing_symbol: cur_class.map(str::to_owned),
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
                        enclosing_symbol: cur_class.map(str::to_owned),
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
                if let Some(owner_ty) =
                    resolve_php_receiver_type_with_self(obj_txt, cur_class, this_props, var_types)
                {
                    let short = owner_ty.rsplit('\\').next().unwrap_or(&owner_ty).to_owned();
                    ctx.push_ref(AstRef {
                        target_name: owner_ty,
                        member: Some(mem_name),
                        context: "field".to_owned(),
                        access: if is_write { "write" } else { "read" }.to_owned(),
                        member_kind: Some("property".to_owned()),
                        owner_type: Some(short),
                        enclosing_symbol: cur_class.map(str::to_owned),
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
// 1b. Blade Template Extraction (`.blade.php`)
// =========================================================================

fn push_blade_view_import(ctx: &mut ExtractCtx<'_>, raw_view: &str, line: u32, evidence: &str) {
    let view = raw_view.trim();
    if view.is_empty() || view.len() > 160 || view.contains(['\n', '\r', ' ', '$', '{', '}']) {
        return;
    }
    let leaf = view
        .rsplit(['.', '/', ':'])
        .next()
        .unwrap_or(view)
        .to_owned();
    ctx.push_import(AstImport {
        path: format!("blade-view:{view}"),
        symbols: if leaf.is_empty() {
            Vec::new()
        } else {
            vec![leaf]
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

fn kebab_to_studly(raw: &str) -> String {
    let mut out = String::new();
    for seg in raw.split(['-', '_']) {
        if seg.is_empty() {
            continue;
        }
        let mut chars = seg.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    out
}

fn find_masked_blade_close(bytes: &[u8], start: usize, close_delim: &[u8]) -> Option<usize> {
    let mut j = start;
    let mut quote: Option<u8> = None;
    let mut inner_depth = 0usize;
    while j < bytes.len() {
        if let Some(q) = quote {
            if bytes[j] == b'\\' {
                j = (j + 2).min(bytes.len());
                continue;
            }
            if bytes[j] == q {
                quote = None;
            }
            j += 1;
            continue;
        }
        if matches!(bytes[j], b'\'' | b'"' | b'`') {
            quote = Some(bytes[j]);
            j += 1;
            continue;
        }
        if inner_depth == 0 && bytes[j..].starts_with(close_delim) {
            return Some(j + close_delim.len());
        }
        if bytes[j] == b'{' {
            inner_depth += 1;
        } else if bytes[j] == b'}' && inner_depth > 0 {
            inner_depth -= 1;
        }
        j += 1;
    }
    None
}

fn mask_blade_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(start) = rest.find("{{--") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 4..];
        let end = after.find("--}}").map(|i| i + 4).unwrap_or(after.len());
        let comment = &rest[start..start + 4 + end];
        for ch in comment.chars() {
            out.push(if ch == '\n' { '\n' } else { ' ' });
        }
        rest = &after[end..];
    }
    out.push_str(rest);

    // Also mask escaped Blade JS/Alpine expressions (`@{{ ... }}` and `@{!! ... !!}`)
    // so neither inline PHP extraction nor regex scanners misattribute them as PHP calls.
    // Respect `@@{{` / `@@{!!` escaped `@` parity (even `@` counts stay live PHP),
    // string quotes, and balanced nested `{ ... }` braces.
    let bytes = out.as_bytes();
    let mut masked = String::with_capacity(out.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'@' {
            let mut at_count = 0usize;
            while i + at_count < bytes.len() && bytes[i + at_count] == b'@' {
                at_count += 1;
            }
            let after_at = i + at_count;
            let close_delim = if bytes[after_at..].starts_with(b"{{") {
                Some(b"}}".as_slice())
            } else if bytes[after_at..].starts_with(b"{!!") {
                Some(b"!!}".as_slice())
            } else {
                None
            };
            if let Some(delim) = close_delim {
                if at_count.is_multiple_of(2) {
                    for _ in 0..at_count {
                        masked.push('@');
                    }
                    i = after_at;
                    continue;
                }
                for _ in 0..(at_count - 1) {
                    masked.push('@');
                }
                let esc_start = after_at - 1;
                let expr_start = after_at + delim.len();
                if let Some(end) = find_masked_blade_close(bytes, expr_start, delim) {
                    for ch in out[esc_start..end].chars() {
                        masked.push(if ch == '\n' { '\n' } else { ' ' });
                    }
                    i = end;
                    continue;
                }
                masked.push('@');
                i = after_at;
                continue;
            }
            for _ in 0..at_count {
                masked.push('@');
            }
            i = after_at;
            continue;
        }
        let ch = out[i..].chars().next().unwrap();
        masked.push(ch);
        i += ch.len_utf8();
    }
    masked
}

struct BladeRegexes {
    view_dir: regex::Regex,
    view_cond_dir: regex::Regex,
    view_first_dir: regex::Regex,
    inject_dir: regex::Regex,
    use_dir: regex::Regex,
    livewire_dir: regex::Regex,
    livewire_class_dir: regex::Regex,
    x_tag: regex::Regex,
    livewire_tag: regex::Regex,
    php_block: regex::Regex,
    route_call: regex::Regex,
    url_call: regex::Regex,
    action_call: regex::Regex,
    static_call: regex::Regex,
    class_const: regex::Regex,
    ident: regex::Regex,
}

fn blade_regexes() -> &'static BladeRegexes {
    use regex::Regex;
    static RE: std::sync::OnceLock<BladeRegexes> = std::sync::OnceLock::new();
    RE.get_or_init(|| BladeRegexes {
        view_dir: Regex::new(
            r#"(?m)@(extends|include|includeIf|component|each)\s*\(\s*['"]([^'"]+)['"]"#,
        )
        .expect("blade view_dir"),
        view_cond_dir: Regex::new(
            r#"(?m)@(?:includeWhen|includeUnless)\s*\([^,]+,\s*['"]([^'"]+)['"]"#,
        )
        .expect("blade view_cond_dir"),
        view_first_dir: Regex::new(r#"(?m)@includeFirst\s*\(\s*\[([^\]]+)\]"#)
            .expect("blade view_first_dir"),
        inject_dir: Regex::new(
            r#"(?m)@inject\s*\(\s*['"][^'"]+['"]\s*,\s*['"]([^'"]+)['"]"#,
        )
        .expect("blade inject_dir"),
        use_dir: Regex::new(
            r#"(?m)@use\s*\(\s*['"]([^'"]+)['"](?:\s*,\s*['"]([^'"]+)['"])?\s*\)"#,
        )
        .expect("blade use_dir"),
        livewire_dir: Regex::new(r#"(?m)@livewire\s*\(\s*['"]([A-Za-z0-9_.:-]+)['"]"#)
            .expect("blade livewire_dir"),
        livewire_class_dir: Regex::new(
            r#"(?m)@livewire\s*\(\s*\\?([A-Za-z_][A-Za-z0-9_\\]+)::class"#,
        )
        .expect("blade livewire_class_dir"),
        x_tag: Regex::new(r#"<x-([A-Za-z0-9_.:-]+)(?:[\s/>]|$)"#).expect("blade x_tag"),
        livewire_tag: Regex::new(r#"<livewire:([A-Za-z0-9_.-]+)(?:[\s/>]|$)"#)
            .expect("blade livewire_tag"),
        php_block: Regex::new(r#"(?s)@php\b\s*(?:\r?\n)?(.*?)@endphp\b"#)
            .expect("blade php_block"),
        route_call: Regex::new(r#"\broute\s*\(\s*['"]([A-Za-z0-9_.:/-]+)['"]"#)
            .expect("blade route_call"),
        url_call: Regex::new(r#"\burl\s*\(\s*['"](/[^'"]*)['"]"#).expect("blade url_call"),
        action_call: Regex::new(
            r#"\baction\s*\(\s*\[\s*\\?([A-Za-z_][A-Za-z0-9_\\]+)::class\s*,\s*['"]([A-Za-z_][A-Za-z0-9_]*)['"]\s*\]"#,
        )
        .expect("blade action_call"),
        static_call: Regex::new(
            r#"\\?([A-Z][A-Za-z0-9_\\]+)::([A-Za-z_][A-Za-z0-9_]*)\s*\("#,
        )
        .expect("blade static_call"),
        class_const: Regex::new(r#"\\?([A-Z][A-Za-z0-9_\\]+)::class\b"#)
            .expect("blade class_const"),
        ident: Regex::new(r#"\b[A-Za-z_][A-Za-z0-9_]{1,95}\b"#).expect("blade ident"),
    })
}

fn line_at_byte(src: &str, byte_offset: usize) -> u32 {
    let clamped = byte_offset.min(src.len());
    (src[..clamped].bytes().filter(|&b| b == b'\n').count() + 1) as u32
}

fn is_escaped_blade_directive(src: &str, at_offset: usize) -> bool {
    at_offset > 0 && src.as_bytes().get(at_offset - 1) == Some(&b'@')
}

fn extract_blade_inline_exprs(masked: &str) -> Vec<(u32, String)> {
    let bytes = masked.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let is_escaped_open = i > 0 && bytes[i - 1] == b'@';
        let (close_delim, open_len) = if !is_escaped_open
            && bytes[i..].starts_with(b"{{")
            && !bytes[i..].starts_with(b"{{--")
        {
            (b"}}".as_slice(), 2usize)
        } else if !is_escaped_open && bytes[i..].starts_with(b"{!!") {
            (b"!!}".as_slice(), 3usize)
        } else {
            i += 1;
            continue;
        };
        let expr_start = i + open_len;
        let mut cursor = expr_start;
        let mut quote: Option<u8> = None;
        let mut escaped = false;
        let mut found_end = None;
        while cursor < bytes.len() {
            let b = bytes[cursor];
            if let Some(q) = quote {
                if escaped {
                    escaped = false;
                } else if b == b'\\' {
                    escaped = true;
                } else if b == q {
                    quote = None;
                }
                cursor += 1;
                continue;
            }
            if b == b'\'' || b == b'"' {
                quote = Some(b);
                cursor += 1;
                continue;
            }
            if bytes[cursor..].starts_with(close_delim) {
                found_end = Some(cursor);
                break;
            }
            cursor += 1;
        }
        if let Some(end) = found_end {
            let body = masked[expr_start..end].trim();
            if !body.is_empty() {
                out.push((line_at_byte(masked, expr_start), body.to_owned()));
            }
            i = end + close_delim.len();
        } else {
            i = expr_start;
        }
    }
    out
}

fn forward_blade_php_snippet(
    ctx: &mut ExtractCtx<'_>,
    body: &str,
    base_line: u32,
    ensure_stmt: bool,
) {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return;
    }
    let wrapped = if ensure_stmt && !trimmed.ends_with(';') && !trimmed.ends_with('}') {
        format!("<?php\n{trimmed};\n")
    } else {
        format!("<?php\n{trimmed}\n")
    };
    if let Some(sub) = parse_file_ir("__blade_inline__.php", &wrapped) {
        let line_shift = base_line.saturating_sub(2);
        for mut decl in sub.declarations {
            decl.start_line = decl.start_line.saturating_add(line_shift);
            decl.end_line = decl.end_line.saturating_add(line_shift);
            ctx.push_decl(
                &decl.entity.kind,
                &decl.entity.name,
                decl.entity.parent.filter(|p| p != "__blade_inline__.php"),
                decl.start_line,
                decl.end_line,
                decl.visibility,
                decl.modifiers,
                decl.signature,
                decl.params_signature,
                decl.body_fingerprint,
            );
        }
        for mut imp in sub.imports {
            imp.line = imp.line.saturating_add(line_shift);
            ctx.push_import(imp);
        }
        for mut r in sub.calls_and_refs {
            r.line = r.line.saturating_add(line_shift);
            ctx.push_ref(r);
        }
        for sem in sub.semantic_edges {
            ctx.push_semantic(
                &sem.kind,
                sem.target,
                sem.source_symbol,
                sem.line.saturating_add(line_shift),
                &sem.evidence,
            );
        }
    }
}

pub(super) fn extract_blade(ctx: &mut ExtractCtx<'_>, root: Node<'_>) {
    extract_php(ctx, root);
    let masked = mask_blade_comments(ctx.source);
    let re = blade_regexes();

    for cap in re.view_dir.captures_iter(&masked) {
        let m = cap.get(0).unwrap();
        if is_escaped_blade_directive(&masked, m.start()) {
            continue;
        }
        let line = line_at_byte(&masked, m.start());
        push_blade_view_import(ctx, &cap[2], line, m.as_str());
    }
    for cap in re.view_cond_dir.captures_iter(&masked) {
        let m = cap.get(0).unwrap();
        if is_escaped_blade_directive(&masked, m.start()) {
            continue;
        }
        let line = line_at_byte(&masked, m.start());
        push_blade_view_import(ctx, &cap[1], line, m.as_str());
    }
    for cap in re.view_first_dir.captures_iter(&masked) {
        let m = cap.get(0).unwrap();
        if is_escaped_blade_directive(&masked, m.start()) {
            continue;
        }
        let line = line_at_byte(&masked, m.start());
        for piece in cap[1].split(',') {
            let trimmed = piece.trim().trim_matches(|c| c == '\'' || c == '"');
            push_blade_view_import(ctx, trimmed, line, m.as_str());
        }
    }
    for cap in re.inject_dir.captures_iter(&masked) {
        let m = cap.get(0).unwrap();
        if is_escaped_blade_directive(&masked, m.start()) {
            continue;
        }
        let line = line_at_byte(&masked, m.start());
        let fqn = cap[1].trim().trim_start_matches('\\').to_owned();
        let short = fqn.rsplit('\\').next().unwrap_or(&fqn).to_owned();
        ctx.push_import(AstImport {
            path: fqn.clone(),
            symbols: vec![short],
            alias: None,
            is_wildcard: false,
            is_reexport: false,
            reexport_visibility: None,
            kind: "import".to_owned(),
            line,
            evidence: clip_str(m.as_str(), 160),
        });
        ctx.push_ref(AstRef {
            target_name: fqn,
            member: None,
            context: "field".to_owned(),
            access: "type".to_owned(),
            member_kind: None,
            owner_type: None,
            enclosing_symbol: None,
            is_static: false,
            line,
            evidence: clip_str(m.as_str(), 160),
        });
    }
    for cap in re.use_dir.captures_iter(&masked) {
        let m = cap.get(0).unwrap();
        if is_escaped_blade_directive(&masked, m.start()) {
            continue;
        }
        let line = line_at_byte(&masked, m.start());
        let fqn = cap[1].trim().trim_start_matches('\\').to_owned();
        let short = fqn.rsplit('\\').next().unwrap_or(&fqn).to_owned();
        let alias = cap.get(2).map(|a| a.as_str().trim().to_owned());
        ctx.push_import(AstImport {
            path: fqn,
            symbols: vec![short],
            alias,
            is_wildcard: false,
            is_reexport: false,
            reexport_visibility: None,
            kind: "import".to_owned(),
            line,
            evidence: clip_str(m.as_str(), 160),
        });
    }
    for cap in re.livewire_dir.captures_iter(&masked) {
        let m = cap.get(0).unwrap();
        if is_escaped_blade_directive(&masked, m.start()) {
            continue;
        }
        let line = line_at_byte(&masked, m.start());
        let comp = cap[1].trim();
        let studly = kebab_to_studly(comp.rsplit(['.', ':']).next().unwrap_or(comp));
        ctx.push_import(AstImport {
            path: format!("blade-livewire:{comp}"),
            symbols: if studly.is_empty() {
                Vec::new()
            } else {
                vec![studly]
            },
            alias: None,
            is_wildcard: false,
            is_reexport: false,
            reexport_visibility: None,
            kind: "include".to_owned(),
            line,
            evidence: clip_str(m.as_str(), 160),
        });
    }
    for cap in re.livewire_class_dir.captures_iter(&masked) {
        let m = cap.get(0).unwrap();
        if is_escaped_blade_directive(&masked, m.start()) {
            continue;
        }
        let line = line_at_byte(&masked, m.start());
        let fqn = cap[1].trim().trim_start_matches('\\').to_owned();
        let short = fqn.rsplit('\\').next().unwrap_or(&fqn).to_owned();
        ctx.push_import(AstImport {
            path: fqn.clone(),
            symbols: vec![short],
            alias: None,
            is_wildcard: false,
            is_reexport: false,
            reexport_visibility: None,
            kind: "import".to_owned(),
            line,
            evidence: clip_str(m.as_str(), 160),
        });
        ctx.push_ref(AstRef {
            target_name: fqn,
            member: None,
            context: "parameter_type".to_owned(),
            access: "type".to_owned(),
            member_kind: None,
            owner_type: None,
            enclosing_symbol: None,
            is_static: true,
            line,
            evidence: clip_str(m.as_str(), 160),
        });
    }
    for cap in re.x_tag.captures_iter(&masked) {
        let tag = cap[1].trim();
        if tag.eq_ignore_ascii_case("slot")
            || tag.to_ascii_lowercase().starts_with("slot:")
            || tag.is_empty()
        {
            continue;
        }
        let m = cap.get(0).unwrap();
        let line = line_at_byte(&masked, m.start());
        let leaf = tag
            .rsplit("::")
            .next()
            .unwrap_or(tag)
            .rsplit('.')
            .next()
            .unwrap_or(tag);
        let studly = kebab_to_studly(leaf);
        ctx.push_import(AstImport {
            path: format!("blade-component:{tag}"),
            symbols: if studly.is_empty() {
                Vec::new()
            } else {
                vec![studly]
            },
            alias: None,
            is_wildcard: false,
            is_reexport: false,
            reexport_visibility: None,
            kind: "include".to_owned(),
            line,
            evidence: clip_str(m.as_str(), 160),
        });
    }
    for cap in re.livewire_tag.captures_iter(&masked) {
        let comp = cap[1].trim();
        if comp.is_empty() {
            continue;
        }
        let m = cap.get(0).unwrap();
        let line = line_at_byte(&masked, m.start());
        let leaf = comp.rsplit('.').next().unwrap_or(comp);
        let studly = kebab_to_studly(leaf);
        ctx.push_import(AstImport {
            path: format!("blade-livewire:{comp}"),
            symbols: if studly.is_empty() {
                Vec::new()
            } else {
                vec![studly]
            },
            alias: None,
            is_wildcard: false,
            is_reexport: false,
            reexport_visibility: None,
            kind: "include".to_owned(),
            line,
            evidence: clip_str(m.as_str(), 160),
        });
    }
    for cap in re.route_call.captures_iter(&masked) {
        let m = cap.get(0).unwrap();
        let line = line_at_byte(&masked, m.start());
        let route_name = cap[1].trim();
        if !route_name.is_empty() {
            let target = if route_name.starts_with('/') {
                normalize_route_contract(Some("ANY"), route_name, true)
                    .unwrap_or_else(|| format!("route:ANY {route_name}"))
            } else {
                format!("route:NAME {route_name}")
            };
            ctx.push_semantic("calls_route", target, None, line, m.as_str());
        }
    }
    for cap in re.url_call.captures_iter(&masked) {
        let m = cap.get(0).unwrap();
        let line = line_at_byte(&masked, m.start());
        if let Some(target) = normalize_route_contract(Some("ANY"), &cap[1], true) {
            ctx.push_semantic("calls_route", target, None, line, m.as_str());
        }
    }
    for cap in re.action_call.captures_iter(&masked) {
        let m = cap.get(0).unwrap();
        let line = line_at_byte(&masked, m.start());
        let ctrl = cap[1].trim().trim_start_matches('\\').to_owned();
        let method = cap[2].trim().to_owned();
        let short = ctrl.rsplit('\\').next().unwrap_or(&ctrl).to_owned();
        ctx.push_ref(AstRef {
            target_name: ctrl,
            member: Some(method),
            context: "call".to_owned(),
            access: "call".to_owned(),
            member_kind: Some("method".to_owned()),
            owner_type: Some(short),
            enclosing_symbol: None,
            is_static: false,
            line,
            evidence: clip_str(m.as_str(), 160),
        });
    }
    for cap in re.static_call.captures_iter(&masked) {
        let m = cap.get(0).unwrap();
        let line = line_at_byte(&masked, m.start());
        let cls = cap[1].trim().trim_start_matches('\\').to_owned();
        let method = cap[2].trim().to_owned();
        let short = cls.rsplit('\\').next().unwrap_or(&cls).to_owned();
        ctx.push_ref(AstRef {
            target_name: cls,
            member: Some(method),
            context: "call".to_owned(),
            access: "call".to_owned(),
            member_kind: Some("method".to_owned()),
            owner_type: Some(short),
            enclosing_symbol: None,
            is_static: true,
            line,
            evidence: clip_str(m.as_str(), 160),
        });
    }
    for cap in re.class_const.captures_iter(&masked) {
        let m = cap.get(0).unwrap();
        let line = line_at_byte(&masked, m.start());
        let cls = cap[1].trim().trim_start_matches('\\').to_owned();
        ctx.push_ref(AstRef {
            target_name: cls,
            member: None,
            context: "parameter_type".to_owned(),
            access: "type".to_owned(),
            member_kind: None,
            owner_type: None,
            enclosing_symbol: None,
            is_static: true,
            line,
            evidence: clip_str(m.as_str(), 160),
        });
    }
    for cap in re.php_block.captures_iter(&masked) {
        let inner_m = cap.get(1).unwrap();
        forward_blade_php_snippet(
            ctx,
            inner_m.as_str(),
            line_at_byte(&masked, inner_m.start()),
            false,
        );
    }
    for (base_line, body) in extract_blade_inline_exprs(&masked) {
        forward_blade_php_snippet(ctx, &body, base_line, true);
    }
}

pub(super) fn collect_blade_identifiers(source: &str, out: &mut BTreeSet<String>) {
    let masked = mask_blade_comments(source);
    let re = blade_regexes();
    for (_, expr_body) in extract_blade_inline_exprs(&masked) {
        for m in re.ident.find_iter(&expr_body) {
            out.insert(m.as_str().to_owned());
        }
    }
    for cap in re.view_dir.captures_iter(&masked) {
        for m in re.ident.find_iter(&cap[2]) {
            out.insert(m.as_str().to_owned());
        }
    }
    for cap in re.view_cond_dir.captures_iter(&masked) {
        for m in re.ident.find_iter(&cap[1]) {
            out.insert(m.as_str().to_owned());
        }
    }
    for cap in re.x_tag.captures_iter(&masked) {
        let tag = cap[1].trim();
        let studly = kebab_to_studly(tag.rsplit(['.', ':']).next().unwrap_or(tag));
        if !studly.is_empty() {
            out.insert(studly);
        }
        for m in re.ident.find_iter(tag) {
            out.insert(m.as_str().to_owned());
        }
    }
    for cap in re.livewire_tag.captures_iter(&masked) {
        let comp = cap[1].trim();
        let studly = kebab_to_studly(comp.rsplit('.').next().unwrap_or(comp));
        if !studly.is_empty() {
            out.insert(studly);
        }
        for m in re.ident.find_iter(comp) {
            out.insert(m.as_str().to_owned());
        }
    }
    for cap in re.livewire_dir.captures_iter(&masked) {
        let comp = cap[1].trim();
        let studly = kebab_to_studly(comp.rsplit(['.', ':']).next().unwrap_or(comp));
        if !studly.is_empty() {
            out.insert(studly);
        }
    }
    for cap in re.route_call.captures_iter(&masked) {
        for m in re.ident.find_iter(&cap[1]) {
            out.insert(m.as_str().to_owned());
        }
    }
    for cap in re.php_block.captures_iter(&masked) {
        for m in re.ident.find_iter(&cap[1]) {
            out.insert(m.as_str().to_owned());
        }
    }
    for cap in re.static_call.captures_iter(&masked) {
        for m in re.ident.find_iter(&cap[1]) {
            out.insert(m.as_str().to_owned());
        }
        out.insert(cap[2].to_owned());
    }
}

// =========================================================================
// 2. Rust Extraction
// =========================================================================
