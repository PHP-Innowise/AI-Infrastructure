use super::*;

pub(super) fn extract_ts_js(ctx: &mut ExtractCtx<'_>, root: Node<'_>) {
    let mut this_props: BTreeMap<String, String> = BTreeMap::new();
    let mut var_types: BTreeMap<String, String> = BTreeMap::new();
    walk_ts_js(
        ctx,
        root,
        0,
        None,
        0,
        false,
        &mut this_props,
        &mut var_types,
    );
}

#[allow(clippy::too_many_arguments)]
fn walk_ts_js(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    depth: usize,
    owner_class: Option<&str>,
    fn_depth: usize,
    is_exported: bool,
    this_props: &mut BTreeMap<String, String>,
    var_types: &mut BTreeMap<String, String>,
) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    let kind = node.kind();

    match kind {
        "import_statement" => {
            extract_ts_import(ctx, node, false);
            return;
        }
        "export_statement" => {
            if let Some(src) = node.child_by_field_name("source") {
                let spec = unquote_literal(ctx.text(src)).unwrap_or_default();
                let line = start_line(node);
                let evidence = ctx.text(node);
                let (syms, is_wc) = collect_ts_export_clause(ctx, node);
                if !spec.is_empty() {
                    ctx.push_import(AstImport {
                        path: spec,
                        symbols: syms,
                        alias: None,
                        is_wildcard: is_wc,
                        is_reexport: true,
                        reexport_visibility: Some("export".to_owned()),
                        kind: "reexport".to_owned(),
                        line,
                        evidence: clip_str(evidence, 160),
                    });
                }
                return;
            }
            if let Some(decl) = node.child_by_field_name("declaration") {
                walk_ts_js(
                    ctx,
                    decl,
                    depth + 1,
                    owner_class,
                    fn_depth,
                    true,
                    this_props,
                    var_types,
                );
                return;
            }
        }
        "class_declaration"
        | "abstract_class_declaration"
        | "interface_declaration"
        | "enum_declaration" => {
            let body_node = node.child_by_field_name("body");
            if fn_depth > 0 {
                if let Some(body) = body_node {
                    let mut nested_this = this_props.clone();
                    let mut nested_vars = var_types.clone();
                    walk_ts_js(
                        ctx,
                        body,
                        depth + 1,
                        None,
                        fn_depth + 1,
                        false,
                        &mut nested_this,
                        &mut nested_vars,
                    );
                }
                return;
            }
            let decl_kind = match kind {
                "interface_declaration" => "interface",
                "enum_declaration" => "enum",
                _ => "class",
            };
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let line = start_line(node);
            let e_line = end_line(node);
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let mut sig = header_without_comments(node, sig_end, ctx.source);
            if is_exported && !sig.starts_with("export ") {
                sig = format!("export {sig}");
            }
            let mut mods = Vec::new();
            if kind == "abstract_class_declaration" {
                push_unique_mod(&mut mods, "abstract");
            }
            let class_route_prefix = extract_ts_class_route_prefix(node, ctx.source);
            emit_ts_decorators(ctx, node, Some(&name));

            if !name.is_empty() {
                ctx.push_decl(
                    decl_kind,
                    &name,
                    Some(ctx.rel_path.clone()),
                    line,
                    e_line,
                    Some(if is_exported { "export" } else { "private" }.to_owned()),
                    mods,
                    sig.clone(),
                    strip_ts_vis_from_sig(&sig),
                    String::new(),
                );
                extract_ts_heritage(ctx, node, &name);
                let attr_mark = class_route_prefix
                    .as_deref()
                    .map(|p| ctx.push_attr_route_prefix(p));
                if let Some(body) = body_node {
                    walk_ts_js(
                        ctx,
                        body,
                        depth + 1,
                        Some(&name),
                        fn_depth,
                        false,
                        this_props,
                        var_types,
                    );
                }
                if let Some(mark) = attr_mark {
                    ctx.truncate_attr_route_prefixes(mark);
                }
                this_props.clear();
                return;
            }
        }
        "type_alias_declaration" => {
            if fn_depth > 0 {
                return;
            }
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = ctx.text(name_node).trim().to_owned();
                let line = start_line(node);
                let e_line = end_line(node);
                let val_node = node.child_by_field_name("value");
                let sig_end = val_node.map_or(node.end_byte(), |v| v.start_byte());
                let mut sig = header_without_comments(node, sig_end, ctx.source)
                    .trim_end_matches(['=', ';'])
                    .trim()
                    .to_owned();
                if is_exported && !sig.starts_with("export ") {
                    sig = format!("export {sig}");
                }
                ctx.push_decl(
                    "type",
                    &name,
                    Some(ctx.rel_path.clone()),
                    line,
                    e_line,
                    Some(if is_exported { "export" } else { "private" }.to_owned()),
                    Vec::new(),
                    sig.clone(),
                    strip_ts_vis_from_sig(&sig),
                    val_node
                        .map(|v| ast_body_fingerprint(v, ctx.source))
                        .unwrap_or_default(),
                );
            }
            return;
        }
        "function_declaration" | "generator_function_declaration" | "function_signature" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let line = start_line(node);
            let e_line = end_line(node);
            let body_node = node.child_by_field_name("body");
            if fn_depth == 0 && !name.is_empty() {
                let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
                let mut sig = header_without_comments(node, sig_end, ctx.source)
                    .trim_end_matches(';')
                    .trim()
                    .to_owned();
                if is_exported && !sig.starts_with("export ") {
                    sig = format!("export {sig}");
                }
                let mods = collect_ts_modifiers(node, ctx.source);
                let body_fp = body_node
                    .map(|b| ast_body_fingerprint(b, ctx.source))
                    .unwrap_or_default();
                ctx.push_decl(
                    "function",
                    &name,
                    Some(ctx.rel_path.clone()),
                    line,
                    e_line,
                    Some(if is_exported { "export" } else { "private" }.to_owned()),
                    mods,
                    sig.clone(),
                    strip_ts_vis_from_sig(&sig),
                    body_fp,
                );
            }
            let mut local_vars = var_types.clone();
            if let Some(params) = node.child_by_field_name("parameters") {
                extract_ts_params(ctx, params, owner_class, false, this_props, &mut local_vars);
            }
            if let Some(body) = body_node {
                walk_ts_js(
                    ctx,
                    body,
                    depth + 1,
                    owner_class,
                    fn_depth + 1,
                    false,
                    this_props,
                    &mut local_vars,
                );
            }
            return;
        }
        "method_definition" | "method_signature" | "abstract_method_signature" => {
            if let Some(owner) = owner_class {
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
                let vis = ts_member_visibility(node, &name, ctx.source);
                let mods = collect_ts_modifiers(node, ctx.source);
                let body_fp = body_node
                    .map(|b| ast_body_fingerprint(b, ctx.source))
                    .unwrap_or_default();
                emit_ts_decorators(ctx, node, Some(&format!("{owner}::{name}")));
                if !name.is_empty() {
                    ctx.push_decl(
                        "method",
                        &name,
                        Some(owner.to_owned()),
                        line,
                        e_line,
                        Some(vis),
                        mods,
                        sig.clone(),
                        strip_ts_vis_from_sig(&sig),
                        body_fp,
                    );
                }
                let mut local_vars = var_types.clone();
                if let Some(params) = node.child_by_field_name("parameters") {
                    extract_ts_params(
                        ctx,
                        params,
                        Some(owner),
                        name == "constructor",
                        this_props,
                        &mut local_vars,
                    );
                }
                if let Some(body) = body_node {
                    walk_ts_js(
                        ctx,
                        body,
                        depth + 1,
                        Some(owner),
                        fn_depth + 1,
                        false,
                        this_props,
                        &mut local_vars,
                    );
                }
                return;
            }
        }
        "public_field_definition" | "field_definition" => {
            if let Some(owner) = owner_class
                && let Some(name_node) = node.child_by_field_name("name")
            {
                let name = ctx.text(name_node).trim().to_owned();
                let line = start_line(node);
                let e_line = end_line(node);
                let val_node = node.child_by_field_name("value");
                let sig_end = val_node.map_or(node.end_byte(), |v| v.start_byte());
                let sig = header_without_comments(node, sig_end, ctx.source)
                    .trim_end_matches(['=', ';'])
                    .trim()
                    .to_owned();
                let vis = ts_member_visibility(node, &name, ctx.source);
                let mods = collect_ts_modifiers(node, ctx.source);
                let body_fp = val_node
                    .map(|v| ast_body_fingerprint(v, ctx.source))
                    .unwrap_or_default();
                if let Some(ty) = node.child_by_field_name("type")
                    && let Some(ty_ident) = extract_ts_type_ident(ctx.text(ty))
                {
                    this_props.insert(name.clone(), ty_ident.clone());
                    ctx.push_ref(AstRef {
                        target_name: ty_ident,
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
                if !name.is_empty() {
                    ctx.push_decl(
                        "property",
                        &name,
                        Some(owner.to_owned()),
                        line,
                        e_line,
                        Some(vis),
                        mods,
                        sig.clone(),
                        strip_ts_vis_from_sig(&sig),
                        body_fp,
                    );
                }
                if let Some(v) = val_node {
                    walk_ts_js(
                        ctx,
                        v,
                        depth + 1,
                        Some(owner),
                        fn_depth,
                        false,
                        this_props,
                        var_types,
                    );
                }
                return;
            }
        }
        "lexical_declaration" | "variable_declaration" => {
            let is_const_decl = ctx.text(node).trim_start().starts_with("const ");
            for decl in children_by_kind(node, "variable_declarator") {
                let Some(name_node) = decl.child_by_field_name("name") else {
                    continue;
                };
                let var_name = ctx.text(name_node).trim().to_owned();
                let val_node = decl.child_by_field_name("value");
                if let Some(ty) = decl
                    .child_by_field_name("type")
                    .and_then(|t| extract_ts_type_ident(ctx.text(t)))
                {
                    var_types.insert(var_name.clone(), ty);
                } else if let Some(val) = val_node
                    && let Some(ty) = infer_ts_expr_type(ctx, val, this_props)
                {
                    var_types.insert(var_name.clone(), ty);
                }

                if fn_depth == 0 && owner_class.is_none() {
                    let is_arrow_or_fn = val_node.is_some_and(|v| {
                        matches!(
                            v.kind(),
                            "arrow_function" | "function_expression" | "generator_function"
                        )
                    });
                    if is_arrow_or_fn {
                        let fn_node = val_node.unwrap();
                        let body_node = fn_node.child_by_field_name("body");
                        let sig_end = body_node.map_or(decl.end_byte(), |b| b.start_byte());
                        let mut sig = header_without_comments(node, sig_end, ctx.source)
                            .trim_end_matches("=>")
                            .trim()
                            .to_owned();
                        if is_exported && !sig.starts_with("export ") {
                            sig = format!("export {sig}");
                        }
                        let body_fp = body_node
                            .map(|b| ast_body_fingerprint(b, ctx.source))
                            .unwrap_or_default();
                        ctx.push_decl(
                            "function",
                            &var_name,
                            Some(ctx.rel_path.clone()),
                            start_line(decl),
                            end_line(decl),
                            Some(if is_exported { "export" } else { "private" }.to_owned()),
                            Vec::new(),
                            sig.clone(),
                            strip_ts_vis_from_sig(&sig),
                            body_fp,
                        );
                    } else if is_const_decl && is_upper_snake(&var_name) {
                        let sig_end = val_node.map_or(decl.end_byte(), |v| v.start_byte());
                        let mut sig = header_without_comments(node, sig_end, ctx.source)
                            .trim_end_matches(['=', ';'])
                            .trim()
                            .to_owned();
                        if is_exported && !sig.starts_with("export ") {
                            sig = format!("export {sig}");
                        }
                        let body_fp = val_node
                            .map(|v| ast_body_fingerprint(v, ctx.source))
                            .unwrap_or_default();
                        ctx.push_decl(
                            "const",
                            &var_name,
                            Some(ctx.rel_path.clone()),
                            start_line(decl),
                            end_line(decl),
                            Some(if is_exported { "export" } else { "private" }.to_owned()),
                            Vec::new(),
                            sig.clone(),
                            strip_ts_vis_from_sig(&sig),
                            body_fp,
                        );
                    }
                }
            }
        }
        "assignment_expression" | "augmented_assignment_expression" => {
            if let (Some(lhs), Some(rhs)) = (
                node.child_by_field_name("left"),
                node.child_by_field_name("right"),
            ) {
                if let Some(ty) = infer_ts_expr_type(ctx, rhs, this_props) {
                    let lhs_txt = ctx.text(lhs).trim();
                    if let Some(prop) = lhs_txt.strip_prefix("this.") {
                        this_props.insert(prop.to_owned(), ty);
                    } else if lhs.kind() == "identifier" {
                        var_types.insert(lhs_txt.to_owned(), ty);
                    }
                }
                if lhs.kind() == "member_expression" {
                    emit_ts_member_expr(ctx, lhs, true, owner_class, this_props, var_types);
                    walk_ts_js(
                        ctx,
                        rhs,
                        depth + 1,
                        owner_class,
                        fn_depth,
                        false,
                        this_props,
                        var_types,
                    );
                    return;
                }
            }
        }
        "new_expression" => {
            if let Some(ctor) = node.child_by_field_name("constructor") {
                let name = ctx
                    .text(ctor)
                    .split('<')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_owned();
                ctx.push_ref(AstRef {
                    target_name: name,
                    member: None,
                    context: "constructor".to_owned(),
                    access: "call".to_owned(),
                    member_kind: None,
                    owner_type: None,
                    enclosing_symbol: owner_class.map(str::to_owned),
                    is_static: false,
                    line: start_line(node),
                    evidence: clip_str(ctx.text(node), 160),
                });
            }
        }
        "call_expression" => {
            if let Some(func_node) = node.child_by_field_name("function") {
                let fn_txt = ctx.text(func_node).trim();
                let line = start_line(node);
                let evidence = ctx.text(node);
                let args_node = node.child_by_field_name("arguments");
                let args_txt = args_node.map(|a| ctx.text(a)).unwrap_or("");
                let str_args = args_node
                    .map(|a| all_string_arg_texts(a, ctx.source))
                    .unwrap_or_default();

                // Dynamic `import('./lazy')` or `require('./mod')`
                if matches!(fn_txt, "import" | "require")
                    && let Some(spec) = str_args.first()
                {
                    let stem = spec
                        .rsplit('/')
                        .next()
                        .unwrap_or(spec)
                        .split('.')
                        .next()
                        .unwrap_or("")
                        .to_owned();
                    ctx.push_import(AstImport {
                        path: spec.clone(),
                        symbols: if stem.is_empty() {
                            Vec::new()
                        } else {
                            vec![stem]
                        },
                        alias: None,
                        is_wildcard: false,
                        is_reexport: false,
                        reexport_visibility: None,
                        kind: "import".to_owned(),
                        line,
                        evidence: clip_str(evidence, 160),
                    });
                }

                if func_node.kind() == "member_expression" {
                    if let (Some(obj), Some(prop)) = (
                        func_node.child_by_field_name("object"),
                        func_node.child_by_field_name("property"),
                    ) {
                        let obj_txt = ctx.text(obj).trim();
                        let prop_txt = ctx.text(prop).trim();
                        let owner_ty = resolve_ts_receiver_type(obj_txt, this_props, var_types);
                        let effective_recv = owner_ty
                            .as_deref()
                            .filter(|ty| {
                                let t = ty.to_ascii_lowercase();
                                t.contains("router") || t.contains("route")
                            })
                            .unwrap_or(obj_txt);

                        // Chained `router.route('/orders').get(fn)` -> compose `/orders` with HTTP verb.
                        let chained_route =
                            chained_express_route_path(obj, prop_txt, &str_args, ctx.source);
                        let (chained_route_leaf, route_root_obj) = match chained_route {
                            Some((leaf, root_obj)) => (Some(leaf), root_obj),
                            None => (None, obj_txt.to_owned()),
                        };
                        let root_owner_ty = if route_root_obj == obj_txt {
                            owner_ty.clone()
                        } else {
                            resolve_ts_receiver_type(&route_root_obj, this_props, var_types)
                        };
                        let effective_route_recv = root_owner_ty
                            .as_deref()
                            .filter(|ty| {
                                let t = ty.to_ascii_lowercase();
                                t.contains("router") || t.contains("route")
                            })
                            .unwrap_or(route_root_obj.as_str());
                        if let Some(leaf_path) = chained_route_leaf.as_deref() {
                            let composed = ctx.compose_call_route_path(leaf_path);
                            let verb = match prop_txt.to_ascii_lowercase().as_str() {
                                "get" => "GET",
                                "post" => "POST",
                                "put" => "PUT",
                                "patch" => "PATCH",
                                "delete" => "DELETE",
                                "options" => "OPTIONS",
                                "head" => "HEAD",
                                _ => "ANY",
                            };
                            if let Some(target) =
                                normalize_route_contract(Some(verb), &composed, false)
                            {
                                ctx.push_semantic(
                                    "exposes_route",
                                    target,
                                    owner_class.map(str::to_owned),
                                    line,
                                    evidence,
                                );
                            }
                        } else {
                            emit_semantic_from_call(
                                ctx,
                                Some(effective_recv),
                                prop_txt,
                                args_txt,
                                &str_args,
                                evidence,
                                owner_class.map(str::to_owned),
                                line,
                            );
                        }

                        // Record simple Express router variable leaves so `app.use('/prefix', routerVar)`
                        // mounts propagate cleanly.
                        let prop_low = prop_txt.to_ascii_lowercase();
                        if is_ts_route_receiver_hint(effective_route_recv)
                            && matches!(
                                prop_low.as_str(),
                                "get"
                                    | "post"
                                    | "put"
                                    | "patch"
                                    | "delete"
                                    | "options"
                                    | "head"
                                    | "all"
                            )
                            && let Some(leaf) = chained_route_leaf
                                .as_deref()
                                .or_else(|| str_args.first().map(String::as_str))
                            && !route_root_obj.is_empty()
                            && route_root_obj
                                .chars()
                                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
                        {
                            let verb = if prop_low == "all" {
                                "ANY".to_owned()
                            } else {
                                prop_low.to_ascii_uppercase()
                            };
                            let leaf_composed = ctx.compose_call_route_path(leaf);
                            ctx.ts_router_leaves
                                .entry(route_root_obj.clone())
                                .or_default()
                                .push((
                                    verb,
                                    leaf_composed,
                                    owner_class.map(str::to_owned),
                                    line,
                                    evidence.to_owned(),
                                ));
                        }

                        let is_route_use =
                            prop_low == "use" && is_ts_route_receiver_hint(effective_recv);
                        let mounted_prefix = if is_route_use {
                            str_args
                                .first()
                                .map(|s| s.trim())
                                .filter(|s| s.starts_with('/'))
                                .map(str::to_owned)
                        } else {
                            None
                        };

                        let target = owner_ty.as_deref().unwrap_or(obj_txt).to_owned();
                        ctx.push_ref(AstRef {
                            target_name: target,
                            member: Some(prop_txt.to_owned()),
                            context: "call".to_owned(),
                            access: "call".to_owned(),
                            member_kind: Some("method".to_owned()),
                            owner_type: owner_ty,
                            enclosing_symbol: owner_class.map(str::to_owned),
                            is_static: false,
                            line,
                            evidence: clip_str(evidence, 160),
                        });

                        if let Some(ref prefix) = mounted_prefix {
                            if let Some(args) = args_node {
                                emit_mounted_router_routes(
                                    ctx,
                                    args,
                                    prefix,
                                    Some(&route_root_obj),
                                );
                                let mark = ctx.push_call_route_prefix(prefix);
                                walk_ts_js(
                                    ctx,
                                    args,
                                    depth + 1,
                                    owner_class,
                                    fn_depth,
                                    false,
                                    this_props,
                                    var_types,
                                );
                                ctx.truncate_call_route_prefixes(mark);
                            }
                            walk_ts_js(
                                ctx,
                                obj,
                                depth + 1,
                                owner_class,
                                fn_depth,
                                false,
                                this_props,
                                var_types,
                            );
                            return;
                        } else if is_route_use
                            && str_args.is_empty()
                            && let Some(args) = args_node
                        {
                            emit_mounted_router_routes(ctx, args, "/", Some(&route_root_obj));
                        }
                    }
                } else {
                    emit_semantic_from_call(
                        ctx,
                        None,
                        fn_txt,
                        args_txt,
                        &str_args,
                        evidence,
                        owner_class.map(str::to_owned),
                        line,
                    );
                    ctx.push_ref(AstRef {
                        target_name: fn_txt.to_owned(),
                        member: None,
                        context: "call".to_owned(),
                        access: "call".to_owned(),
                        member_kind: Some("function".to_owned()),
                        owner_type: None,
                        enclosing_symbol: owner_class.map(str::to_owned),
                        is_static: false,
                        line,
                        evidence: clip_str(evidence, 160),
                    });
                }
                if let Some(args) = args_node {
                    walk_ts_js(
                        ctx,
                        args,
                        depth + 1,
                        owner_class,
                        fn_depth,
                        false,
                        this_props,
                        var_types,
                    );
                }
                if func_node.kind() == "member_expression"
                    && let Some(obj) = func_node.child_by_field_name("object")
                {
                    walk_ts_js(
                        ctx,
                        obj,
                        depth + 1,
                        owner_class,
                        fn_depth,
                        false,
                        this_props,
                        var_types,
                    );
                }
                return;
            }
        }
        "member_expression" => {
            emit_ts_member_expr(ctx, node, false, owner_class, this_props, var_types);
        }
        "required_parameter" | "optional_parameter" => {
            emit_ts_decorators(ctx, node, owner_class);
        }
        "object" => {
            let mut has_provide = false;
            let mut targets = Vec::new();
            for pair in children_by_kind(node, "pair") {
                let Some(k_node) = pair.child_by_field_name("key") else {
                    continue;
                };
                let key = ctx
                    .text(k_node)
                    .trim()
                    .trim_matches(|c| c == '\'' || c == '"');
                if !matches!(key, "provide" | "useClass" | "useExisting") {
                    continue;
                }
                if key == "provide" {
                    has_provide = true;
                }
                if let Some(v_node) = pair.child_by_field_name("value") {
                    let raw_v = ctx.text(v_node).trim();
                    let tok = unquote_literal(raw_v).unwrap_or_else(|| {
                        raw_v
                            .split(['<', '.', ' '])
                            .next()
                            .unwrap_or("")
                            .trim()
                            .to_owned()
                    });
                    if !tok.is_empty() && !targets.contains(&tok) {
                        targets.push(tok);
                    }
                }
            }
            if has_provide {
                let line = start_line(node);
                let evidence = ctx.text(node);
                for svc in targets {
                    ctx.push_semantic(
                        "di_binds",
                        format!("service:{svc}"),
                        owner_class.map(str::to_owned),
                        line,
                        evidence,
                    );
                }
            }
        }
        _ => {}
    }

    for i in 0..node.child_count() {
        if let Some(ch) = node.child(i) {
            walk_ts_js(
                ctx,
                ch,
                depth + 1,
                owner_class,
                fn_depth,
                false,
                this_props,
                var_types,
            );
        }
    }
}

fn extract_ts_import(ctx: &mut ExtractCtx<'_>, node: Node<'_>, is_reexport: bool) {
    let Some(src) = node.child_by_field_name("source") else {
        return;
    };
    let Some(spec) = unquote_literal(ctx.text(src)) else {
        return;
    };
    let line = start_line(node);
    let evidence = ctx.text(node);

    let Some(clause) = child_by_kind(node, "import_clause") else {
        ctx.push_import(AstImport {
            path: spec,
            symbols: Vec::new(),
            alias: None,
            is_wildcard: false,
            is_reexport,
            reexport_visibility: None,
            kind: "import".to_owned(),
            line,
            evidence: clip_str(evidence, 160),
        });
        return;
    };

    let mut emitted_any = false;
    for i in 0..clause.child_count() {
        let Some(ch) = clause.child(i) else {
            continue;
        };
        match ch.kind() {
            "identifier" => {
                let name = ctx.text(ch).trim().to_owned();
                emitted_any = true;
                ctx.push_import(AstImport {
                    path: spec.clone(),
                    symbols: vec![name],
                    alias: None,
                    is_wildcard: false,
                    is_reexport,
                    reexport_visibility: None,
                    kind: "import".to_owned(),
                    line,
                    evidence: clip_str(evidence, 160),
                });
            }
            "namespace_import" => {
                let ns = child_by_kind(ch, "identifier").map(|n| ctx.text(n).trim().to_owned());
                emitted_any = true;
                ctx.push_import(AstImport {
                    path: spec.clone(),
                    symbols: Vec::new(),
                    alias: ns,
                    is_wildcard: true,
                    is_reexport,
                    reexport_visibility: None,
                    kind: "import".to_owned(),
                    line,
                    evidence: clip_str(evidence, 160),
                });
            }
            "named_imports" => {
                for spec_node in children_by_kind(ch, "import_specifier") {
                    let orig = spec_node
                        .child_by_field_name("name")
                        .map(|n| ctx.text(n).trim().to_owned())
                        .unwrap_or_default();
                    let alias = spec_node
                        .child_by_field_name("alias")
                        .map(|n| ctx.text(n).trim().to_owned());
                    if !orig.is_empty() {
                        emitted_any = true;
                        ctx.push_import(AstImport {
                            path: spec.clone(),
                            symbols: vec![orig],
                            alias,
                            is_wildcard: false,
                            is_reexport,
                            reexport_visibility: None,
                            kind: "import".to_owned(),
                            line,
                            evidence: clip_str(evidence, 160),
                        });
                    }
                }
            }
            _ => {}
        }
    }
    if !emitted_any {
        ctx.push_import(AstImport {
            path: spec,
            symbols: Vec::new(),
            alias: None,
            is_wildcard: false,
            is_reexport,
            reexport_visibility: None,
            kind: "import".to_owned(),
            line,
            evidence: clip_str(evidence, 160),
        });
    }
}

fn collect_ts_export_clause(ctx: &ExtractCtx<'_>, node: Node<'_>) -> (Vec<String>, bool) {
    let mut syms = Vec::new();
    let mut is_wc = false;
    for i in 0..node.child_count() {
        let Some(ch) = node.child(i) else {
            continue;
        };
        if ch.kind() == "export_clause" {
            for spec in children_by_kind(ch, "export_specifier") {
                if let Some(n) = spec.child_by_field_name("name") {
                    syms.push(ctx.text(n).trim().to_owned());
                }
            }
        } else if ch.kind() == "namespace_export" || ctx.text(ch).trim() == "*" {
            is_wc = true;
        }
    }
    (syms, is_wc)
}

fn extract_ts_heritage(ctx: &mut ExtractCtx<'_>, class_node: Node<'_>, class_name: &str) {
    let Some(heritage) = child_by_kind(class_node, "class_heritage") else {
        return;
    };
    let line = start_line(heritage);
    let evidence = ctx.text(heritage);
    for i in 0..heritage.child_count() {
        let Some(ch) = heritage.child(i) else {
            continue;
        };
        match ch.kind() {
            "extends_clause" => {
                if let Some(val) = ch.child_by_field_name("value") {
                    let base = ctx
                        .text(val)
                        .split('<')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_owned();
                    ctx.push_inheritance(class_name, &base, "extends", line, evidence);
                }
            }
            "implements_clause" => {
                for j in 0..ch.child_count() {
                    if let Some(item) = ch.child(j)
                        && item.is_named()
                    {
                        let iface = ctx
                            .text(item)
                            .split('<')
                            .next()
                            .unwrap_or("")
                            .trim()
                            .to_owned();
                        ctx.push_inheritance(class_name, &iface, "implements", line, evidence);
                    }
                }
            }
            _ => {}
        }
    }
}

fn is_ts_route_receiver_hint(recv: &str) -> bool {
    let r = recv.trim().to_ascii_lowercase();
    r.contains("route")
        || r.contains("app")
        || r.contains("mux")
        || r.contains("fastify")
        || r.contains("server")
        || r.contains("hono")
        || matches!(r.as_str(), "r" | "api")
}

fn chained_express_route_path(
    obj: Node<'_>,
    callee: &str,
    str_args: &[String],
    source: &str,
) -> Option<(String, String)> {
    if !str_args.is_empty() {
        return None;
    }
    let cal_low = callee.to_ascii_lowercase();
    if !matches!(
        cal_low.as_str(),
        "get" | "post" | "put" | "patch" | "delete" | "options" | "head" | "all"
    ) {
        return None;
    }
    let mut cur = Some(obj);
    while let Some(node) = cur {
        if node.kind() != "call_expression" {
            break;
        }
        let Some(func) = node.child_by_field_name("function") else {
            break;
        };
        if func.kind() != "member_expression" {
            break;
        }
        let Some(prop) = func.child_by_field_name("property") else {
            break;
        };
        if node_text(prop, source).trim().eq_ignore_ascii_case("route")
            && let Some(args) = node.child_by_field_name("arguments")
            && let Some(path) = all_string_arg_texts(args, source).into_iter().next()
        {
            let root = func
                .child_by_field_name("object")
                .map(|r| node_text(r, source).trim().to_owned())
                .unwrap_or_default();
            return Some((path, root));
        }
        cur = func.child_by_field_name("object");
    }
    None
}

fn emit_mounted_router_routes(
    ctx: &mut ExtractCtx<'_>,
    args: Node<'_>,
    prefix: &str,
    parent_router: Option<&str>,
) {
    let parent_valid = parent_router.filter(|p| {
        !p.is_empty()
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
    });
    let mut mounted_leaves = Vec::new();
    for i in 0..args.named_child_count() {
        let Some(ch) = args.named_child(i) else {
            continue;
        };
        if ch.kind() == "identifier" {
            let ident = ctx.text(ch).trim();
            if parent_valid == Some(ident) {
                continue;
            }
            if let Some(leaves) = ctx.ts_router_leaves.get(ident) {
                mounted_leaves.extend(leaves.iter().cloned());
            }
        }
    }
    if mounted_leaves.is_empty() {
        return;
    }
    let mark = ctx.push_call_route_prefix(prefix);
    let mut propagated = Vec::with_capacity(mounted_leaves.len());
    for (verb, leaf_path, enclosing, line, evidence) in mounted_leaves {
        let composed = ctx.compose_call_route_path(&leaf_path);
        if let Some(target) = normalize_route_contract(Some(&verb), &composed, false) {
            ctx.push_semantic("exposes_route", target, enclosing.clone(), line, &evidence);
        }
        propagated.push((verb, composed, enclosing, line, evidence));
    }
    ctx.truncate_call_route_prefixes(mark);
    if let Some(parent) = parent_valid {
        ctx.ts_router_leaves
            .entry(parent.to_owned())
            .or_default()
            .extend(propagated);
    }
}

fn extract_ts_class_route_prefix(node: Node<'_>, source: &str) -> Option<String> {
    let check_container = |container: Node<'_>| -> Option<String> {
        for deco in children_by_kind(container, "decorator") {
            let txt = node_text(deco, source);
            let name = txt
                .trim_start_matches('@')
                .split('(')
                .next()
                .unwrap_or("")
                .trim()
                .rsplit(['.', ':'])
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            if matches!(
                name.as_str(),
                "controller" | "route" | "requestmapping" | "path"
            ) && let Some(first) = all_string_arg_texts(deco, source)
                .into_iter()
                .find(|s| !s.trim().is_empty())
            {
                return Some(first);
            }
        }
        None
    };
    if let Some(found) = check_container(node) {
        return Some(found);
    }
    if let Some(parent) = node.parent()
        && parent.kind() == "export_statement"
    {
        return check_container(parent);
    }
    None
}

fn emit_ts_decorators(ctx: &mut ExtractCtx<'_>, node: Node<'_>, sym: Option<&str>) {
    let mut emit_from = |container: Node<'_>| {
        for deco in children_by_kind(container, "decorator") {
            let txt = ctx.text(deco);
            let strs = all_string_arg_texts(deco, ctx.source);
            let name = txt
                .trim_start_matches('@')
                .split('(')
                .next()
                .unwrap_or("")
                .trim();
            emit_semantic_from_attribute(ctx, name, txt, &strs, sym, &[], start_line(deco));
        }
    };
    emit_from(node);
    if let Some(parent) = node.parent()
        && parent.kind() == "export_statement"
    {
        emit_from(parent);
    }
}

fn ts_member_visibility(node: Node<'_>, name: &str, source: &str) -> String {
    if name.starts_with('#') {
        return "#private".to_owned();
    }
    if let Some(acc) = child_by_kind(node, "accessibility_modifier") {
        return node_text(acc, source).trim().to_owned();
    }
    "public".to_owned()
}

fn collect_ts_modifiers(node: Node<'_>, source: &str) -> Vec<String> {
    let mut mods = Vec::new();
    for i in 0..node.child_count() {
        if let Some(ch) = node.child(i) {
            let txt = node_text(ch, source).trim();
            if matches!(
                txt,
                "static"
                    | "readonly"
                    | "abstract"
                    | "async"
                    | "override"
                    | "declare"
                    | "get"
                    | "set"
            ) {
                push_unique_mod(&mut mods, txt);
            }
        }
    }
    mods
}

fn strip_ts_vis_from_sig(sig: &str) -> String {
    let mut parts = Vec::new();
    let mut skipping_leading = true;
    for tok in sig.split_whitespace() {
        if skipping_leading && matches!(tok, "export" | "public" | "protected" | "private") {
            continue;
        }
        skipping_leading = false;
        parts.push(tok);
    }
    parts.join(" ")
}

fn extract_ts_type_ident(raw: &str) -> Option<String> {
    let cleaned = raw
        .trim()
        .trim_start_matches(':')
        .split(['=', '|', '<', '&'])
        .next()?
        .trim();
    if cleaned
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_uppercase())
    {
        Some(cleaned.to_owned())
    } else {
        None
    }
}

fn extract_ts_params(
    ctx: &mut ExtractCtx<'_>,
    params_node: Node<'_>,
    owner_class: Option<&str>,
    is_ctor: bool,
    this_props: &mut BTreeMap<String, String>,
    var_types: &mut BTreeMap<String, String>,
) {
    for i in 0..params_node.child_count() {
        let Some(p) = params_node.child(i) else {
            continue;
        };
        if !matches!(p.kind(), "required_parameter" | "optional_parameter") {
            continue;
        }
        emit_ts_decorators(ctx, p, owner_class);
        let pname = p
            .child_by_field_name("pattern")
            .or_else(|| p.child_by_field_name("name"))
            .map(|n| ctx.text(n).trim().to_owned())
            .unwrap_or_default();
        let ty_ident = p
            .child_by_field_name("type")
            .and_then(|t| extract_ts_type_ident(ctx.text(t)));
        if let Some(ref ty) = ty_ident {
            if !pname.is_empty() {
                var_types.insert(pname.clone(), ty.clone());
            }
            ctx.push_ref(AstRef {
                target_name: ty.clone(),
                member: None,
                context: "parameter_type".to_owned(),
                access: "type".to_owned(),
                member_kind: None,
                owner_type: None,
                enclosing_symbol: owner_class.map(str::to_owned),
                is_static: false,
                line: start_line(p),
                evidence: clip_str(ctx.text(p), 160),
            });
        }
        if is_ctor && let Some(owner) = owner_class {
            let acc =
                child_by_kind(p, "accessibility_modifier").map(|a| ctx.text(a).trim().to_owned());
            let has_readonly = (0..p.child_count())
                .filter_map(|idx| p.child(idx))
                .any(|c| ctx.text(c).trim() == "readonly");
            if acc.is_some() || has_readonly {
                let vis = acc.unwrap_or_else(|| "public".to_owned());
                let mut mods = Vec::new();
                if has_readonly {
                    mods.push("readonly".to_owned());
                }
                if let Some(ref ty) = ty_ident {
                    this_props.insert(pname.clone(), ty.clone());
                }
                let val_node = p.child_by_field_name("value");
                let sig_end = val_node.map_or(p.end_byte(), |v| v.start_byte());
                let sig = header_without_comments(p, sig_end, ctx.source)
                    .trim_end_matches('=')
                    .trim()
                    .to_owned();
                ctx.push_decl(
                    "property",
                    &pname,
                    Some(owner.to_owned()),
                    start_line(p),
                    end_line(p),
                    Some(vis),
                    mods,
                    sig.clone(),
                    strip_ts_vis_from_sig(&sig),
                    val_node
                        .map(|v| ast_body_fingerprint(v, ctx.source))
                        .unwrap_or_default(),
                );
            }
        }
    }
}

fn infer_ts_expr_type(
    ctx: &ExtractCtx<'_>,
    rhs: Node<'_>,
    this_props: &BTreeMap<String, String>,
) -> Option<String> {
    match rhs.kind() {
        "new_expression" => rhs
            .child_by_field_name("constructor")
            .map(|c| {
                ctx.text(c)
                    .split('<')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_owned()
            })
            .filter(|s| !s.is_empty()),
        "call_expression" => rhs.child_by_field_name("function").and_then(|f| {
            let fn_low = ctx.text(f).trim().to_ascii_lowercase();
            if fn_low.ends_with(".router")
                || fn_low == "router"
                || fn_low == "express"
                || fn_low == "fastify"
                || fn_low == "hono"
            {
                Some("ExpressRouter".to_owned())
            } else {
                None
            }
        }),
        "member_expression" => {
            let txt = ctx.text(rhs).trim();
            if let Some(prop) = txt.strip_prefix("this.") {
                return this_props.get(prop.trim()).cloned();
            }
            None
        }
        _ => None,
    }
}

fn resolve_ts_receiver_type(
    recv: &str,
    this_props: &BTreeMap<String, String>,
    var_types: &BTreeMap<String, String>,
) -> Option<String> {
    let trimmed = recv.trim();
    if let Some(prop) = trimmed
        .strip_prefix("this.")
        .or_else(|| trimmed.strip_prefix("this?."))
    {
        return this_props.get(prop.trim()).cloned();
    }
    var_types.get(trimmed).cloned()
}

fn emit_ts_member_expr(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    is_write: bool,
    owner_class: Option<&str>,
    this_props: &BTreeMap<String, String>,
    var_types: &BTreeMap<String, String>,
) {
    let Some(obj) = node.child_by_field_name("object") else {
        return;
    };
    let Some(prop) = node.child_by_field_name("property") else {
        return;
    };
    let obj_txt = ctx.text(obj).trim();
    let prop_txt = ctx.text(prop).trim();
    let owner_ty = resolve_ts_receiver_type(obj_txt, this_props, var_types);
    let target = owner_ty.clone().unwrap_or_else(|| obj_txt.to_owned());
    let is_static = target
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_uppercase())
        && owner_ty.is_none();
    let is_const = !is_write && is_upper_snake(prop_txt);
    ctx.push_ref(AstRef {
        target_name: target,
        member: Some(prop_txt.to_owned()),
        context: "field".to_owned(),
        access: if is_write {
            "write"
        } else if is_const {
            "const"
        } else {
            "read"
        }
        .to_owned(),
        member_kind: Some(if is_const { "const" } else { "property" }.to_owned()),
        owner_type: owner_ty.or_else(|| is_static.then(|| obj_txt.to_owned())),
        enclosing_symbol: owner_class.map(str::to_owned),
        is_static,
        line: start_line(node),
        evidence: clip_str(ctx.text(node), 160),
    });
}

// =========================================================================
// 4. Python Extraction
// =========================================================================
