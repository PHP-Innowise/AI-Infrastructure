use super::*;

pub(super) fn extract_rust(ctx: &mut ExtractCtx<'_>, root: Node<'_>) {
    let mut struct_fields: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut var_types: BTreeMap<String, String> = BTreeMap::new();
    walk_rust(
        ctx,
        root,
        0,
        None,
        false,
        0,
        &mut struct_fields,
        &mut var_types,
    );
}

#[allow(clippy::too_many_arguments)]
fn walk_rust(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    depth: usize,
    owner_name: Option<&str>,
    owner_is_trait_or_trait_impl: bool,
    fn_depth: usize,
    struct_fields: &mut BTreeMap<String, BTreeMap<String, String>>,
    var_types: &mut BTreeMap<String, String>,
) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    let kind = node.kind();

    match kind {
        "use_declaration" => {
            extract_rust_use_decl(ctx, node);
            return;
        }
        "mod_item" => {
            let body = node.child_by_field_name("body");
            if body.is_none()
                && let Some(name_node) = node.child_by_field_name("name")
            {
                let mod_name = ctx.text(name_node).trim().to_owned();
                let attr_path = find_preceding_rust_path_attr(node, ctx.source);
                let line = start_line(node);
                let path = attr_path.unwrap_or_else(|| mod_name.clone());
                ctx.push_import(AstImport {
                    path,
                    symbols: vec![mod_name],
                    alias: None,
                    is_wildcard: false,
                    is_reexport: false,
                    reexport_visibility: None,
                    kind: "mod_decl".to_owned(),
                    line,
                    evidence: clip_str(ctx.text(node), 160),
                });
            } else if let Some(b) = body {
                if let Some(attr_line) = has_preceding_rust_test_attr(node, ctx.source) {
                    ctx.test_spans.push((attr_line, end_line(node)));
                }
                walk_rust(
                    ctx,
                    b,
                    depth + 1,
                    owner_name,
                    false,
                    fn_depth,
                    struct_fields,
                    var_types,
                );
            }
            return;
        }
        "struct_item" | "enum_item" | "trait_item" => {
            if fn_depth > 0 {
                return;
            }
            let decl_kind = match kind {
                "struct_item" => "struct",
                "enum_item" => "enum",
                _ => "trait",
            };
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
            let vis = rust_visibility(node, ctx.source).unwrap_or_else(|| "private".to_owned());
            let params_sig = strip_rust_vis_from_sig(&sig);

            if !name.is_empty() {
                ctx.push_decl(
                    decl_kind,
                    &name,
                    Some(ctx.rel_path.clone()),
                    line,
                    e_line,
                    Some(vis),
                    Vec::new(),
                    sig,
                    params_sig,
                    String::new(),
                );
                if let Some(body) = body_node {
                    walk_rust(
                        ctx,
                        body,
                        depth + 1,
                        Some(&name),
                        decl_kind == "trait",
                        fn_depth,
                        struct_fields,
                        var_types,
                    );
                }
            }
            return;
        }
        "impl_item" => {
            if fn_depth > 0 {
                return;
            }
            let type_node = node.child_by_field_name("type");
            let trait_node = node.child_by_field_name("trait");
            let target_ty = type_node
                .map(|n| short_rust_type(ctx.text(n)))
                .unwrap_or_default();
            let line = start_line(node);
            let evidence = ctx.text(node);
            if let Some(tn) = trait_node {
                let trait_ty = short_rust_type(ctx.text(tn));
                ctx.push_inheritance(&target_ty, &trait_ty, "implements", line, evidence);
                ctx.push_ref(AstRef {
                    target_name: ctx.text(tn).trim().to_owned(),
                    member: None,
                    context: "parameter_type".to_owned(),
                    access: "type".to_owned(),
                    member_kind: None,
                    owner_type: None,
                    enclosing_symbol: None,
                    is_static: false,
                    line,
                    evidence: clip_str(evidence, 160),
                });
            }
            if let Some(ty_n) = type_node {
                ctx.push_ref(AstRef {
                    target_name: unwrap_rust_type(ctx.text(ty_n)),
                    member: None,
                    context: "parameter_type".to_owned(),
                    access: "type".to_owned(),
                    member_kind: None,
                    owner_type: None,
                    enclosing_symbol: None,
                    is_static: false,
                    line,
                    evidence: clip_str(evidence, 160),
                });
            }
            if let Some(body) = node.child_by_field_name("body") {
                walk_rust(
                    ctx,
                    body,
                    depth + 1,
                    (!target_ty.is_empty()).then_some(target_ty.as_str()),
                    trait_node.is_some(),
                    fn_depth,
                    struct_fields,
                    var_types,
                );
            }
            return;
        }
        "field_declaration" => {
            if let Some(owner) = owner_name {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| ctx.text(n).trim().to_owned())
                    .unwrap_or_default();
                let line = start_line(node);
                let e_line = end_line(node);
                let sig = header_without_comments(node, node.end_byte(), ctx.source)
                    .trim_end_matches(',')
                    .trim()
                    .to_owned();
                let vis = rust_visibility(node, ctx.source).unwrap_or_else(|| "private".to_owned());
                let params_sig = strip_rust_vis_from_sig(&sig);
                if let Some(ty) = node.child_by_field_name("type") {
                    let unwrapped = unwrap_rust_type(ctx.text(ty));
                    if !unwrapped.is_empty() && !name.is_empty() {
                        struct_fields
                            .entry(owner.to_owned())
                            .or_default()
                            .insert(name.clone(), unwrapped.clone());
                        ctx.push_ref(AstRef {
                            target_name: unwrapped,
                            member: None,
                            context: "field".to_owned(),
                            access: "type".to_owned(),
                            member_kind: None,
                            owner_type: None,
                            enclosing_symbol: Some(owner.to_owned()),
                            is_static: false,
                            line,
                            evidence: clip_str(&sig, 160),
                        });
                    }
                }
                if !name.is_empty() {
                    ctx.push_decl(
                        "field",
                        &name,
                        Some(owner.to_owned()),
                        line,
                        e_line,
                        Some(vis),
                        Vec::new(),
                        sig,
                        params_sig,
                        String::new(),
                    );
                }
            }
            return;
        }
        "const_item" | "static_item" => {
            if fn_depth > 0 {
                return;
            }
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let line = start_line(node);
            let e_line = end_line(node);
            let val_node = node.child_by_field_name("value");
            let sig_end = val_node.map_or(node.end_byte(), |v| v.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source)
                .trim_end_matches(['=', ';'])
                .trim()
                .to_owned();
            let vis = rust_visibility(node, ctx.source).unwrap_or_else(|| {
                if owner_is_trait_or_trait_impl {
                    "public".to_owned()
                } else {
                    "private".to_owned()
                }
            });
            let params_sig = strip_rust_vis_from_sig(&sig);
            let body_fp = val_node
                .map(|v| ast_body_fingerprint(v, ctx.source))
                .unwrap_or_default();
            let mut mods = Vec::new();
            if kind == "static_item" {
                push_unique_mod(&mut mods, "static");
                if child_by_kind(node, "mutable_specifier").is_some() {
                    push_unique_mod(&mut mods, "mut");
                }
            }
            if !name.is_empty() {
                ctx.push_decl(
                    "const",
                    &name,
                    owner_name.map(str::to_owned),
                    line,
                    e_line,
                    Some(vis),
                    mods,
                    sig,
                    params_sig,
                    body_fp,
                );
            }
            if let Some(val) = val_node {
                walk_rust(
                    ctx,
                    val,
                    depth + 1,
                    owner_name,
                    owner_is_trait_or_trait_impl,
                    fn_depth,
                    struct_fields,
                    var_types,
                );
            }
            return;
        }
        "function_item" | "function_signature_item" => {
            if fn_depth > 0 {
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
            let vis = rust_visibility(node, ctx.source).unwrap_or_else(|| {
                if owner_is_trait_or_trait_impl {
                    "public".to_owned()
                } else {
                    "private".to_owned()
                }
            });
            let params_sig = strip_rust_vis_from_sig(&sig);
            let body_fp = body_node
                .map(|b| ast_body_fingerprint(b, ctx.source))
                .unwrap_or_default();
            let mods = collect_rust_fn_modifiers(node, ctx.source);
            let decl_kind = if owner_name.is_some() {
                "method"
            } else {
                "function"
            };
            if !name.is_empty() {
                if let Some(attr_line) = has_preceding_rust_test_attr(node, ctx.source) {
                    ctx.test_spans.push((attr_line, e_line));
                }
                ctx.push_decl(
                    decl_kind,
                    &name,
                    owner_name.map(str::to_owned),
                    line,
                    e_line,
                    Some(vis),
                    mods,
                    sig,
                    params_sig,
                    body_fp,
                );
            }
            let mut local_vars = var_types.clone();
            if let Some(params) = node.child_by_field_name("parameters") {
                extract_rust_params(ctx, params, owner_name, &mut local_vars);
            }
            if let Some(ret) = node.child_by_field_name("return_type") {
                let ty = unwrap_rust_type(ctx.text(ret).trim_start_matches("->"));
                if !ty.is_empty() {
                    ctx.push_ref(AstRef {
                        target_name: ty,
                        member: None,
                        context: "return_type".to_owned(),
                        access: "type".to_owned(),
                        member_kind: None,
                        owner_type: None,
                        enclosing_symbol: owner_name.map(str::to_owned),
                        is_static: false,
                        line: start_line(ret),
                        evidence: clip_str(ctx.text(ret), 160),
                    });
                }
            }
            if let Some(body) = body_node {
                walk_rust(
                    ctx,
                    body,
                    depth + 1,
                    owner_name,
                    owner_is_trait_or_trait_impl,
                    fn_depth + 1,
                    struct_fields,
                    &mut local_vars,
                );
            }
            return;
        }
        "let_declaration" => {
            if let Some(pat) = node.child_by_field_name("pattern") {
                let var_name = ctx
                    .text(pat)
                    .trim()
                    .trim_start_matches("mut ")
                    .trim()
                    .to_owned();
                let mut inferred = node
                    .child_by_field_name("type")
                    .map(|t| unwrap_rust_type(ctx.text(t)))
                    .filter(|s| !s.is_empty());
                if inferred.is_none()
                    && let Some(val) = node.child_by_field_name("value")
                {
                    inferred = infer_rust_expr_type(
                        ctx,
                        val,
                        owner_name.and_then(|o| struct_fields.get(o)),
                    );
                }
                if let Some(ty) = inferred
                    && !var_name.is_empty()
                {
                    var_types.insert(var_name, ty);
                }
            }
        }
        "macro_invocation" => {
            if let Some(mac) = node.child_by_field_name("macro") {
                let mac_txt = ctx.text(mac).trim();
                let short = mac_txt.rsplit("::").next().unwrap_or(mac_txt);
                if short == "table" {
                    let full = ctx.text(node);
                    if let Some(brace_start) = full.find('{') {
                        let inner = full[brace_start + 1..].trim();
                        if let Some(tbl) = inner
                            .split(|c: char| c.is_whitespace() || c == '(')
                            .find(|s| !s.is_empty())
                            .and_then(clean_sql_table_name)
                        {
                            ctx.push_semantic(
                                "reads_table",
                                format!("table:{tbl}"),
                                owner_name.map(str::to_owned),
                                start_line(node),
                                full,
                            );
                        }
                    }
                }
            }
        }
        "attribute_item" | "inner_attribute_item" => {
            let txt = ctx.text(node);
            if let Some(attr) = child_by_kind(node, "attribute") {
                let name = attr.child(0).map(|n| ctx.text(n)).unwrap_or(txt);
                let strs = all_string_arg_texts(attr, ctx.source);
                emit_semantic_from_attribute(
                    ctx,
                    name,
                    txt,
                    &strs,
                    owner_name,
                    &[],
                    start_line(node),
                );
            }
        }
        "assignment_expression" | "compound_assignment_expr" => {
            if let Some(lhs) = node.child_by_field_name("left")
                && lhs.kind() == "field_expression"
            {
                emit_rust_field_expr(ctx, lhs, true, owner_name, struct_fields, var_types);
                if let Some(rhs) = node.child_by_field_name("right") {
                    walk_rust(
                        ctx,
                        rhs,
                        depth + 1,
                        owner_name,
                        owner_is_trait_or_trait_impl,
                        fn_depth,
                        struct_fields,
                        var_types,
                    );
                }
                return;
            }
        }
        "struct_expression" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let struct_ty = ctx.text(name_node).trim().to_owned();
                let short_ty = short_rust_type(&struct_ty);
                let line = start_line(node);
                let evidence = ctx.text(node);
                ctx.push_ref(AstRef {
                    target_name: struct_ty.clone(),
                    member: None,
                    context: "constructor".to_owned(),
                    access: "type".to_owned(),
                    member_kind: None,
                    owner_type: Some(short_ty.clone()),
                    enclosing_symbol: owner_name.map(str::to_owned),
                    is_static: false,
                    line,
                    evidence: clip_str(evidence, 160),
                });
                if let Some(body) = node.child_by_field_name("body") {
                    for init in children_by_kind(body, "field_initializer") {
                        if let Some(fname) = init.child_by_field_name("field") {
                            ctx.push_ref(AstRef {
                                target_name: struct_ty.clone(),
                                member: Some(ctx.text(fname).trim().to_owned()),
                                context: "field".to_owned(),
                                access: "write_init".to_owned(),
                                member_kind: Some("field".to_owned()),
                                owner_type: Some(short_ty.clone()),
                                enclosing_symbol: owner_name.map(str::to_owned),
                                is_static: false,
                                line: start_line(init),
                                evidence: clip_str(ctx.text(init), 160),
                            });
                        }
                    }
                }
            }
        }
        "call_expression" => {
            if let Some(func_node) = node.child_by_field_name("function") {
                let line = start_line(node);
                let evidence = ctx.text(node);
                let args_node = node.child_by_field_name("arguments");
                let args_txt = args_node.map(|a| ctx.text(a)).unwrap_or("");
                let str_args = args_node
                    .map(|a| all_string_arg_texts(a, ctx.source))
                    .unwrap_or_default();

                match func_node.kind() {
                    "field_expression" => {
                        if let (Some(val_node), Some(field_node)) = (
                            func_node.child_by_field_name("value"),
                            func_node.child_by_field_name("field"),
                        ) {
                            let recv_txt = ctx.text(val_node).trim();
                            let mem_txt = ctx.text(field_node).trim();
                            emit_semantic_from_call(
                                ctx,
                                Some(recv_txt),
                                mem_txt,
                                args_txt,
                                &str_args,
                                evidence,
                                owner_name.map(str::to_owned),
                                line,
                            );
                            if let Some(owner_ty) = resolve_rust_receiver_type(
                                recv_txt,
                                owner_name.and_then(|o| struct_fields.get(o)),
                                var_types,
                            ) {
                                let short = short_rust_type(&owner_ty);
                                ctx.push_ref(AstRef {
                                    target_name: owner_ty,
                                    member: Some(mem_txt.to_owned()),
                                    context: "call".to_owned(),
                                    access: "call".to_owned(),
                                    member_kind: Some("method".to_owned()),
                                    owner_type: Some(short),
                                    enclosing_symbol: owner_name.map(str::to_owned),
                                    is_static: false,
                                    line,
                                    evidence: clip_str(evidence, 160),
                                });
                            }
                        }
                    }
                    "scoped_identifier" | "generic_function" => {
                        let fn_txt = ctx.text(func_node).split("::<").next().unwrap_or("").trim();
                        if let Some((scope, mem)) = fn_txt.rsplit_once("::") {
                            emit_semantic_from_call(
                                ctx,
                                Some(scope),
                                mem,
                                args_txt,
                                &str_args,
                                evidence,
                                owner_name.map(str::to_owned),
                                line,
                            );
                            let short = short_rust_type(scope);
                            let is_type =
                                short.chars().next().is_some_and(|c| c.is_ascii_uppercase());
                            ctx.push_ref(AstRef {
                                target_name: fn_txt.to_owned(),
                                member: Some(mem.to_owned()),
                                context: "call".to_owned(),
                                access: "call".to_owned(),
                                member_kind: Some(
                                    if is_type { "method" } else { "function" }.to_owned(),
                                ),
                                owner_type: is_type.then_some(short),
                                enclosing_symbol: owner_name.map(str::to_owned),
                                is_static: true,
                                line,
                                evidence: clip_str(evidence, 160),
                            });
                        } else if !fn_txt.is_empty() {
                            emit_semantic_from_call(
                                ctx,
                                None,
                                fn_txt,
                                args_txt,
                                &str_args,
                                evidence,
                                owner_name.map(str::to_owned),
                                line,
                            );
                            ctx.push_ref(AstRef {
                                target_name: fn_txt.to_owned(),
                                member: None,
                                context: "call".to_owned(),
                                access: "call".to_owned(),
                                member_kind: Some("function".to_owned()),
                                owner_type: None,
                                enclosing_symbol: owner_name.map(str::to_owned),
                                is_static: false,
                                line,
                                evidence: clip_str(evidence, 160),
                            });
                        }
                    }
                    "identifier" => {
                        let fn_txt = ctx.text(func_node).trim();
                        emit_semantic_from_call(
                            ctx,
                            None,
                            fn_txt,
                            args_txt,
                            &str_args,
                            evidence,
                            owner_name.map(str::to_owned),
                            line,
                        );
                        ctx.push_ref(AstRef {
                            target_name: fn_txt.to_owned(),
                            member: None,
                            context: "call".to_owned(),
                            access: "call".to_owned(),
                            member_kind: Some("function".to_owned()),
                            owner_type: None,
                            enclosing_symbol: owner_name.map(str::to_owned),
                            is_static: false,
                            line,
                            evidence: clip_str(evidence, 160),
                        });
                    }
                    _ => {}
                }
                if let Some(args) = args_node {
                    walk_rust(
                        ctx,
                        args,
                        depth + 1,
                        owner_name,
                        owner_is_trait_or_trait_impl,
                        fn_depth,
                        struct_fields,
                        var_types,
                    );
                }
                // Walk receiver inside `field_expression` in case of chained calls like `Router::new().route(...)`
                if func_node.kind() == "field_expression"
                    && let Some(val_node) = func_node.child_by_field_name("value")
                {
                    walk_rust(
                        ctx,
                        val_node,
                        depth + 1,
                        owner_name,
                        owner_is_trait_or_trait_impl,
                        fn_depth,
                        struct_fields,
                        var_types,
                    );
                }
                return;
            }
        }
        "field_expression" => {
            emit_rust_field_expr(ctx, node, false, owner_name, struct_fields, var_types);
        }
        "scoped_identifier" => {
            // Skip child of call_expression/use_declaration/etc. (already handled by parent)
            let parent_kind = node.parent().map(|p| p.kind()).unwrap_or("");
            if !matches!(
                parent_kind,
                "call_expression"
                    | "generic_function"
                    | "use_declaration"
                    | "scoped_use_list"
                    | "use_as_clause"
                    | "use_wildcard"
                    | "scoped_identifier"
                    | "struct_expression"
            ) {
                let txt = ctx.text(node).trim();
                if let Some((scope, mem)) = txt.rsplit_once("::") {
                    let short = short_rust_type(scope);
                    let is_const = is_upper_snake(mem);
                    ctx.push_ref(AstRef {
                        target_name: txt.to_owned(),
                        member: is_const.then(|| mem.to_owned()),
                        context: "field".to_owned(),
                        access: if is_const { "const" } else { "type" }.to_owned(),
                        member_kind: is_const.then(|| "const".to_owned()),
                        owner_type: Some(short),
                        enclosing_symbol: owner_name.map(str::to_owned),
                        is_static: true,
                        line: start_line(node),
                        evidence: clip_str(txt, 160),
                    });
                }
            }
        }
        _ => {}
    }

    for i in 0..node.child_count() {
        if let Some(ch) = node.child(i) {
            walk_rust(
                ctx,
                ch,
                depth + 1,
                owner_name,
                owner_is_trait_or_trait_impl,
                fn_depth,
                struct_fields,
                var_types,
            );
        }
    }
}

fn extract_rust_use_decl(ctx: &mut ExtractCtx<'_>, node: Node<'_>) {
    let line = start_line(node);
    let evidence = ctx.text(node);
    let vis = rust_visibility(node, ctx.source);
    let is_reexport = vis.is_some();
    let Some(arg) = node.child_by_field_name("argument") else {
        return;
    };
    let mut expanded: Vec<(Vec<String>, Option<String>, bool)> = Vec::new();
    expand_rust_use_node(arg, ctx.source, &[], &mut expanded);
    for (segs, alias, is_wildcard) in expanded {
        if segs.is_empty() {
            continue;
        }
        let path = segs.join("::");
        let symbols = if is_wildcard {
            Vec::new()
        } else {
            let last = segs.last().cloned().unwrap_or_default();
            if last == "self" {
                segs.iter().rev().nth(1).cloned().into_iter().collect()
            } else {
                vec![last]
            }
        };
        ctx.push_import(AstImport {
            path,
            symbols,
            alias,
            is_wildcard,
            is_reexport,
            reexport_visibility: vis.clone(),
            kind: if is_reexport { "reexport" } else { "import" }.to_owned(),
            line,
            evidence: clip_str(evidence, 160),
        });
    }
}

fn expand_rust_use_node(
    node: Node<'_>,
    source: &str,
    prefix: &[String],
    out: &mut Vec<(Vec<String>, Option<String>, bool)>,
) {
    match node.kind() {
        "identifier" | "crate" | "super" | "self" | "metavariable" => {
            let mut segs = prefix.to_vec();
            segs.push(node_text(node, source).trim().to_owned());
            out.push((segs, None, false));
        }
        "scoped_identifier" => {
            let mut segs = prefix.to_vec();
            for part in node_text(node, source).split("::") {
                let p = part.trim();
                if !p.is_empty() {
                    segs.push(p.to_owned());
                }
            }
            out.push((segs, None, false));
        }
        "use_wildcard" => {
            let mut segs = prefix.to_vec();
            let txt = node_text(node, source).trim();
            if let Some(before) = txt.strip_suffix("::*") {
                for part in before.split("::") {
                    let p = part.trim();
                    if !p.is_empty() {
                        segs.push(p.to_owned());
                    }
                }
            }
            if segs.is_empty() {
                segs.push("*".to_owned());
            }
            out.push((segs, None, true));
        }
        "use_as_clause" => {
            let alias = node
                .child_by_field_name("alias")
                .map(|a| node_text(a, source).trim().to_owned());
            if let Some(path_node) = node.child_by_field_name("path") {
                let mut inner = Vec::new();
                expand_rust_use_node(path_node, source, prefix, &mut inner);
                for (segs, _, is_wc) in inner {
                    out.push((segs, alias.clone(), is_wc));
                }
            }
        }
        "scoped_use_list" => {
            let mut next_prefix = prefix.to_vec();
            if let Some(path_node) = node.child_by_field_name("path") {
                for part in node_text(path_node, source).split("::") {
                    let p = part.trim();
                    if !p.is_empty() {
                        next_prefix.push(p.to_owned());
                    }
                }
            }
            if let Some(list_node) = node.child_by_field_name("list") {
                expand_rust_use_node(list_node, source, &next_prefix, out);
            }
        }
        "use_list" => {
            for i in 0..node.child_count() {
                if let Some(ch) = node.child(i)
                    && ch.is_named()
                {
                    expand_rust_use_node(ch, source, prefix, out);
                }
            }
        }
        _ => {}
    }
}

fn rust_visibility(node: Node<'_>, source: &str) -> Option<String> {
    let vis = child_by_kind(node, "visibility_modifier")?;
    let compact = node_text(vis, source)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if compact == "pub" {
        Some("public".to_owned())
    } else {
        Some(compact.replace("pub (", "pub("))
    }
}

fn strip_rust_vis_from_sig(sig: &str) -> String {
    let trimmed = sig.trim();
    if let Some(rest) = trimmed.strip_prefix("pub")
        && !rest
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
    {
        let after_ws = rest.trim_start();
        if let Some(in_paren) = after_ws.strip_prefix('(') {
            let mut depth = 1usize;
            for (idx, ch) in in_paren.char_indices() {
                match ch {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            return in_paren[idx + 1..].trim_start().to_owned();
                        }
                    }
                    _ => {}
                }
            }
        }
        return after_ws.to_owned();
    }
    trimmed.to_owned()
}

fn collect_rust_fn_modifiers(node: Node<'_>, source: &str) -> Vec<String> {
    let mut mods = Vec::new();
    if let Some(fm) = child_by_kind(node, "function_modifiers") {
        for word in node_text(fm, source).split_whitespace() {
            if matches!(word, "async" | "unsafe" | "const" | "extern") {
                push_unique_mod(&mut mods, word);
            }
        }
    }
    mods
}

fn find_preceding_rust_path_attr(node: Node<'_>, source: &str) -> Option<String> {
    let mut prev = node.prev_sibling();
    while let Some(sib) = prev {
        if is_comment_kind(sib.kind()) {
            prev = sib.prev_sibling();
            continue;
        }
        if sib.kind() != "attribute_item" {
            break;
        }
        let txt = node_text(sib, source);
        if txt.contains("path")
            && txt.contains('=')
            && let Some(val) = first_string_arg_text(sib, source)
        {
            return Some(val);
        }
        prev = sib.prev_sibling();
    }
    None
}

fn has_preceding_rust_test_attr(node: Node<'_>, source: &str) -> Option<u32> {
    let mut prev = node.prev_sibling();
    let mut earliest_test_line: Option<u32> = None;
    while let Some(sib) = prev {
        if is_comment_kind(sib.kind()) {
            prev = sib.prev_sibling();
            continue;
        }
        if sib.kind() != "attribute_item" {
            break;
        }
        let compact: String = node_text(sib, source)
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        let is_test_attr = compact == "#[test]"
            || compact.starts_with("#[test(")
            || compact.contains("::test]")
            || compact.contains("::test(")
            || compact.contains("cfg(test)")
            || compact == "#[rstest]"
            || compact.starts_with("#[rstest(");
        if is_test_attr {
            earliest_test_line = Some(start_line(sib));
        }
        prev = sib.prev_sibling();
    }
    earliest_test_line
}

fn unwrap_rust_type(raw: &str) -> String {
    let mut ty = raw.trim().trim_end_matches([',', ';', ')']).trim();
    loop {
        let before = ty;
        while let Some(rest) = ty.strip_prefix('&') {
            ty = rest.trim_start();
            if ty.starts_with('\'')
                && let Some((_, after)) = ty.split_once(char::is_whitespace)
            {
                ty = after.trim_start();
            }
            if let Some(after) = ty.strip_prefix("mut")
                && after.starts_with(char::is_whitespace)
            {
                ty = after.trim_start();
            }
        }
        for wrapper in [
            "Arc",
            "Rc",
            "Box",
            "Option",
            "Mutex",
            "RwLock",
            "RefCell",
            "Cell",
            "Pin",
            "std::sync::Arc",
            "alloc::rc::Rc",
            "std::boxed::Box",
            "core::option::Option",
        ] {
            if let Some(after_w) = ty.strip_prefix(wrapper)
                && let Some(inner_start) = after_w.trim_start().strip_prefix('<')
                && let Some(last_gt) = inner_start.rfind('>')
            {
                ty = inner_start[..last_gt].trim();
                break;
            }
        }
        if ty == before {
            break;
        }
    }
    ty.split('<').next().unwrap_or(ty).trim().to_owned()
}

fn short_rust_type(raw: &str) -> String {
    let unwrapped = unwrap_rust_type(raw);
    unwrapped
        .rsplit("::")
        .next()
        .unwrap_or(&unwrapped)
        .trim()
        .to_owned()
}

fn extract_rust_params(
    ctx: &mut ExtractCtx<'_>,
    params_node: Node<'_>,
    owner: Option<&str>,
    var_types: &mut BTreeMap<String, String>,
) {
    for param in children_by_kind(params_node, "parameter") {
        if let (Some(pat), Some(ty_node)) = (
            param.child_by_field_name("pattern"),
            param.child_by_field_name("type"),
        ) {
            let pname = ctx
                .text(pat)
                .trim()
                .trim_start_matches("mut ")
                .trim()
                .to_owned();
            let ty_unwrapped = unwrap_rust_type(ctx.text(ty_node));
            if !ty_unwrapped.is_empty() {
                if !pname.is_empty() && pname != "self" {
                    var_types.insert(pname, ty_unwrapped.clone());
                }
                ctx.push_ref(AstRef {
                    target_name: ty_unwrapped,
                    member: None,
                    context: "parameter_type".to_owned(),
                    access: "type".to_owned(),
                    member_kind: None,
                    owner_type: None,
                    enclosing_symbol: owner.map(str::to_owned),
                    is_static: false,
                    line: start_line(param),
                    evidence: clip_str(ctx.text(param), 160),
                });
            }
        }
    }
}

fn infer_rust_expr_type(
    ctx: &ExtractCtx<'_>,
    rhs: Node<'_>,
    self_fields: Option<&BTreeMap<String, String>>,
) -> Option<String> {
    match rhs.kind() {
        "struct_expression" => rhs
            .child_by_field_name("name")
            .map(|n| ctx.text(n).trim().to_owned()),
        "field_expression" => {
            let txt = ctx.text(rhs).trim();
            if let Some(field) = txt.strip_prefix("self.") {
                return self_fields.and_then(|m| m.get(field.trim()).cloned());
            }
            None
        }
        "call_expression" => {
            let fn_txt = rhs
                .child_by_field_name("function")
                .map(|n| ctx.text(n).trim())?;
            if let Some((ty, ctor)) = fn_txt.rsplit_once("::")
                && matches!(
                    ctor,
                    "new" | "default" | "create" | "open" | "connect" | "build" | "from"
                )
            {
                return Some(ty.to_owned());
            }
            None
        }
        _ => None,
    }
}

fn resolve_rust_receiver_type(
    recv: &str,
    self_fields: Option<&BTreeMap<String, String>>,
    var_types: &BTreeMap<String, String>,
) -> Option<String> {
    let trimmed = recv.trim();
    if let Some(field) = trimmed.strip_prefix("self.") {
        return self_fields.and_then(|m| m.get(field.trim()).cloned());
    }
    var_types.get(trimmed).cloned()
}

fn emit_rust_field_expr(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    is_write: bool,
    owner_name: Option<&str>,
    struct_fields: &BTreeMap<String, BTreeMap<String, String>>,
    var_types: &BTreeMap<String, String>,
) {
    let Some(val) = node.child_by_field_name("value") else {
        return;
    };
    let Some(field) = node.child_by_field_name("field") else {
        return;
    };
    let recv_txt = ctx.text(val).trim();
    let field_txt = ctx.text(field).trim().to_owned();
    let self_fields = owner_name.and_then(|o| struct_fields.get(o));
    if let Some(owner_ty) = resolve_rust_receiver_type(recv_txt, self_fields, var_types) {
        let short = short_rust_type(&owner_ty);
        ctx.push_ref(AstRef {
            target_name: owner_ty,
            member: Some(field_txt),
            context: "field".to_owned(),
            access: if is_write { "write" } else { "read" }.to_owned(),
            member_kind: Some("field".to_owned()),
            owner_type: Some(short),
            enclosing_symbol: owner_name.map(str::to_owned),
            is_static: false,
            line: start_line(node),
            evidence: clip_str(ctx.text(node), 160),
        });
    }
}

// =========================================================================
// 3. TypeScript / TSX / JavaScript / JSX Extraction
// =========================================================================
