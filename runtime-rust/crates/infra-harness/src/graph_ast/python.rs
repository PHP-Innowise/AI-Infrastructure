use super::*;

pub(super) fn extract_python(ctx: &mut ExtractCtx<'_>, root: Node<'_>) {
    let mut this_attrs: BTreeMap<String, String> = BTreeMap::new();
    let mut var_types: BTreeMap<String, String> = BTreeMap::new();
    walk_python(ctx, root, 0, None, 0, &[], &mut this_attrs, &mut var_types);
}

#[allow(clippy::too_many_arguments)]
fn walk_python(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    depth: usize,
    owner_class: Option<&str>,
    fn_depth: usize,
    decorators: &[String],
    this_attrs: &mut BTreeMap<String, String>,
    var_types: &mut BTreeMap<String, String>,
) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    let kind = node.kind();

    match kind {
        "import_statement" => {
            let line = start_line(node);
            let evidence = ctx.text(node);
            for i in 0..node.child_count() {
                let Some(ch) = node.child(i) else {
                    continue;
                };
                match ch.kind() {
                    "dotted_name" => {
                        let p = ctx.text(ch).trim().to_owned();
                        let sym = p.rsplit('.').next().unwrap_or(&p).to_owned();
                        ctx.push_import(AstImport {
                            path: p,
                            symbols: vec![sym],
                            alias: None,
                            is_wildcard: false,
                            is_reexport: false,
                            reexport_visibility: None,
                            kind: "import".to_owned(),
                            line,
                            evidence: clip_str(evidence, 160),
                        });
                    }
                    "aliased_import" => {
                        let p = ch
                            .child_by_field_name("name")
                            .map(|n| ctx.text(n).trim().to_owned())
                            .unwrap_or_default();
                        let alias = ch
                            .child_by_field_name("alias")
                            .map(|n| ctx.text(n).trim().to_owned());
                        let sym = p.rsplit('.').next().unwrap_or(&p).to_owned();
                        ctx.push_import(AstImport {
                            path: p,
                            symbols: vec![sym],
                            alias,
                            is_wildcard: false,
                            is_reexport: false,
                            reexport_visibility: None,
                            kind: "import".to_owned(),
                            line,
                            evidence: clip_str(evidence, 160),
                        });
                    }
                    _ => {}
                }
            }
            return;
        }
        "import_from_statement" => {
            let line = start_line(node);
            let evidence = ctx.text(node);
            let mod_name = node
                .child_by_field_name("module_name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let is_wc = child_by_kind(node, "wildcard_import").is_some();
            if is_wc {
                ctx.push_import(AstImport {
                    path: mod_name,
                    symbols: Vec::new(),
                    alias: None,
                    is_wildcard: true,
                    is_reexport: false,
                    reexport_visibility: None,
                    kind: "import".to_owned(),
                    line,
                    evidence: clip_str(evidence, 160),
                });
                return;
            }
            let mut emitted = false;
            for i in 0..node.child_count() {
                let Some(ch) = node.child(i) else {
                    continue;
                };
                if Some(ch) == node.child_by_field_name("module_name") {
                    continue;
                }
                match ch.kind() {
                    "dotted_name" => {
                        let sym = ctx.text(ch).trim().to_owned();
                        emitted = true;
                        ctx.push_import(AstImport {
                            path: mod_name.clone(),
                            symbols: vec![sym],
                            alias: None,
                            is_wildcard: false,
                            is_reexport: false,
                            reexport_visibility: None,
                            kind: "import".to_owned(),
                            line,
                            evidence: clip_str(evidence, 160),
                        });
                    }
                    "aliased_import" => {
                        let sym = ch
                            .child_by_field_name("name")
                            .map(|n| ctx.text(n).trim().to_owned())
                            .unwrap_or_default();
                        let alias = ch
                            .child_by_field_name("alias")
                            .map(|n| ctx.text(n).trim().to_owned());
                        if !sym.is_empty() {
                            emitted = true;
                            ctx.push_import(AstImport {
                                path: mod_name.clone(),
                                symbols: vec![sym],
                                alias,
                                is_wildcard: false,
                                is_reexport: false,
                                reexport_visibility: None,
                                kind: "import".to_owned(),
                                line,
                                evidence: clip_str(evidence, 160),
                            });
                        }
                    }
                    _ => {}
                }
            }
            if !emitted && !mod_name.is_empty() {
                ctx.push_import(AstImport {
                    path: mod_name,
                    symbols: Vec::new(),
                    alias: None,
                    is_wildcard: false,
                    is_reexport: false,
                    reexport_visibility: None,
                    kind: "import".to_owned(),
                    line,
                    evidence: clip_str(evidence, 160),
                });
            }
            return;
        }
        "decorated_definition" => {
            let mut decos = Vec::new();
            for d in children_by_kind(node, "decorator") {
                let raw = ctx.text(d);
                let name = raw
                    .trim_start_matches('@')
                    .split('(')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_owned();
                let strs = all_string_arg_texts(d, ctx.source);
                emit_semantic_from_attribute(
                    ctx,
                    &name,
                    raw,
                    &strs,
                    owner_class,
                    &[],
                    start_line(d),
                );
                let short = name.rsplit('.').next().unwrap_or(&name).to_owned();
                decos.push(short);
            }
            if let Some(def) = node.child_by_field_name("definition") {
                walk_python(
                    ctx,
                    def,
                    depth + 1,
                    owner_class,
                    fn_depth,
                    &decos,
                    this_attrs,
                    var_types,
                );
            }
            return;
        }
        "class_definition" => {
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
                .trim_end_matches(':')
                .trim()
                .to_owned();
            if !name.is_empty() {
                ctx.push_decl(
                    "class",
                    &name,
                    Some(ctx.rel_path.clone()),
                    line,
                    e_line,
                    Some(py_visibility(&name)),
                    Vec::new(),
                    sig.clone(),
                    sig,
                    String::new(),
                );
                if let Some(supers) = node.child_by_field_name("superclasses") {
                    for i in 0..supers.child_count() {
                        if let Some(ch) = supers.child(i)
                            && ch.is_named()
                        {
                            let base = ctx
                                .text(ch)
                                .split('[')
                                .next()
                                .unwrap_or("")
                                .trim()
                                .to_owned();
                            ctx.push_inheritance(&name, &base, "extends", line, ctx.text(supers));
                        }
                    }
                }
                if let Some(body) = body_node {
                    walk_python(
                        ctx,
                        body,
                        depth + 1,
                        Some(&name),
                        fn_depth,
                        &[],
                        this_attrs,
                        var_types,
                    );
                }
                this_attrs.clear();
            }
            return;
        }
        "function_definition" => {
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
                .trim_end_matches(':')
                .trim()
                .to_owned();
            let body_fp = body_node
                .map(|b| ast_body_fingerprint(b, ctx.source))
                .unwrap_or_default();
            let is_prop = decorators.iter().any(|d| d == "property");
            let mut mods: Vec<String> = decorators
                .iter()
                .filter(|d| {
                    matches!(
                        d.as_str(),
                        "property" | "classmethod" | "staticmethod" | "abstractmethod" | "final"
                    )
                })
                .cloned()
                .collect();
            if sig.starts_with("async ") {
                push_unique_mod(&mut mods, "async");
            }
            let decl_kind = if owner_class.is_some() {
                if is_prop { "property" } else { "method" }
            } else {
                "function"
            };
            if !name.is_empty() {
                ctx.push_decl(
                    decl_kind,
                    &name,
                    owner_class.map(str::to_owned),
                    line,
                    e_line,
                    Some(py_visibility(&name)),
                    mods,
                    sig.clone(),
                    sig,
                    body_fp,
                );
            }
            let mut local_vars = var_types.clone();
            if let Some(params) = node.child_by_field_name("parameters") {
                extract_py_params(ctx, params, owner_class, &mut local_vars);
            }
            if let Some(body) = body_node {
                walk_python(
                    ctx,
                    body,
                    depth + 1,
                    owner_class,
                    fn_depth + 1,
                    &[],
                    this_attrs,
                    &mut local_vars,
                );
            }
            return;
        }
        "assignment" => {
            if let Some(lhs) = node.child_by_field_name("left") {
                let lhs_txt = ctx.text(lhs).trim().to_owned();
                let rhs = node.child_by_field_name("right");
                let ty_node = node.child_by_field_name("type");
                let inferred_ty = ty_node
                    .and_then(|t| clean_py_type_str(ctx.text(t)))
                    .or_else(|| {
                        rhs.and_then(|r| infer_py_expr_type(ctx, r, this_attrs, var_types))
                    });

                if let Some(ref ty) = inferred_ty {
                    if let Some(attr) = lhs_txt.strip_prefix("self.") {
                        this_attrs.insert(attr.trim().to_owned(), ty.clone());
                    } else if lhs.kind() == "identifier" {
                        if owner_class.is_some() && fn_depth == 0 {
                            this_attrs.insert(lhs_txt.clone(), ty.clone());
                        } else {
                            var_types.insert(lhs_txt.clone(), ty.clone());
                        }
                    }
                }

                // Class property or module-level constant declarations
                if let Some(owner) = owner_class {
                    if let Some(attr) = lhs_txt.strip_prefix("self.") {
                        let attr_clean = attr.trim();
                        if !attr_clean.is_empty() && !py_decl_exists(ctx, owner, attr_clean) {
                            let line = start_line(node);
                            let sig_end = rhs.map_or(node.end_byte(), |r| r.start_byte());
                            let sig = header_without_comments(node, sig_end, ctx.source)
                                .trim_end_matches('=')
                                .trim()
                                .to_owned();
                            ctx.push_decl(
                                "property",
                                attr_clean,
                                Some(owner.to_owned()),
                                line,
                                end_line(node),
                                Some(py_visibility(attr_clean)),
                                Vec::new(),
                                sig.clone(),
                                sig,
                                rhs.map(|r| ast_body_fingerprint(r, ctx.source))
                                    .unwrap_or_default(),
                            );
                        }
                    } else if fn_depth == 0
                        && lhs.kind() == "identifier"
                        && !py_decl_exists(ctx, owner, &lhs_txt)
                    {
                        let sig_end = rhs.map_or(node.end_byte(), |r| r.start_byte());
                        let sig = header_without_comments(node, sig_end, ctx.source)
                            .trim_end_matches('=')
                            .trim()
                            .to_owned();
                        ctx.push_decl(
                            "property",
                            &lhs_txt,
                            Some(owner.to_owned()),
                            start_line(node),
                            end_line(node),
                            Some(py_visibility(&lhs_txt)),
                            Vec::new(),
                            sig.clone(),
                            sig,
                            rhs.map(|r| ast_body_fingerprint(r, ctx.source))
                                .unwrap_or_default(),
                        );
                    }
                } else if fn_depth == 0 && lhs.kind() == "identifier" && is_upper_snake(&lhs_txt) {
                    let sig_end = rhs.map_or(node.end_byte(), |r| r.start_byte());
                    let sig = header_without_comments(node, sig_end, ctx.source)
                        .trim_end_matches('=')
                        .trim()
                        .to_owned();
                    ctx.push_decl(
                        "const",
                        &lhs_txt,
                        Some(ctx.rel_path.clone()),
                        start_line(node),
                        end_line(node),
                        Some(py_visibility(&lhs_txt)),
                        Vec::new(),
                        sig.clone(),
                        sig,
                        rhs.map(|r| ast_body_fingerprint(r, ctx.source))
                            .unwrap_or_default(),
                    );
                }

                if lhs.kind() == "attribute" {
                    emit_py_attr(ctx, lhs, true, owner_class, this_attrs, var_types);
                    if let Some(r) = rhs {
                        walk_python(
                            ctx,
                            r,
                            depth + 1,
                            owner_class,
                            fn_depth,
                            &[],
                            this_attrs,
                            var_types,
                        );
                    }
                    return;
                }
            }
        }
        "call" => {
            if let Some(func_node) = node.child_by_field_name("function") {
                let line = start_line(node);
                let evidence = ctx.text(node);
                let args_node = node.child_by_field_name("arguments");
                let args_txt = args_node.map(|a| ctx.text(a)).unwrap_or("");
                let str_args = args_node
                    .map(|a| all_string_arg_texts(a, ctx.source))
                    .unwrap_or_default();

                if func_node.kind() == "attribute" {
                    if let (Some(obj), Some(attr)) = (
                        func_node.child_by_field_name("object"),
                        func_node.child_by_field_name("attribute"),
                    ) {
                        let obj_txt = ctx.text(obj).trim();
                        let attr_txt = ctx.text(attr).trim();
                        emit_semantic_from_call(
                            ctx,
                            Some(obj_txt),
                            attr_txt,
                            args_txt,
                            &str_args,
                            evidence,
                            owner_class.map(str::to_owned),
                            line,
                        );
                        let owner_ty = resolve_py_receiver_type(obj_txt, this_attrs, var_types);
                        let target = owner_ty.clone().unwrap_or_else(|| obj_txt.to_owned());
                        ctx.push_ref(AstRef {
                            target_name: target,
                            member: Some(attr_txt.to_owned()),
                            context: "call".to_owned(),
                            access: "call".to_owned(),
                            member_kind: Some("method".to_owned()),
                            owner_type: owner_ty,
                            enclosing_symbol: owner_class.map(str::to_owned),
                            is_static: false,
                            line,
                            evidence: clip_str(evidence, 160),
                        });
                    }
                } else {
                    let fn_txt = ctx.text(func_node).trim();
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
                    walk_python(
                        ctx,
                        args,
                        depth + 1,
                        owner_class,
                        fn_depth,
                        &[],
                        this_attrs,
                        var_types,
                    );
                }
                if func_node.kind() == "attribute"
                    && let Some(obj) = func_node.child_by_field_name("object")
                {
                    walk_python(
                        ctx,
                        obj,
                        depth + 1,
                        owner_class,
                        fn_depth,
                        &[],
                        this_attrs,
                        var_types,
                    );
                }
                return;
            }
        }
        "attribute" => {
            emit_py_attr(ctx, node, false, owner_class, this_attrs, var_types);
        }
        _ => {}
    }

    for i in 0..node.child_count() {
        if let Some(ch) = node.child(i) {
            walk_python(
                ctx,
                ch,
                depth + 1,
                owner_class,
                fn_depth,
                &[],
                this_attrs,
                var_types,
            );
        }
    }
}

fn py_decl_exists(ctx: &ExtractCtx<'_>, owner: &str, name: &str) -> bool {
    ctx.declarations.iter().any(|d| {
        d.entity.parent.as_deref() == Some(owner)
            && d.entity.name == name
            && d.entity.kind == "property"
    })
}

fn py_visibility(name: &str) -> String {
    if name.starts_with("__") && !name.ends_with("__") {
        "private".to_owned()
    } else if name.starts_with('_') && !name.starts_with("__") {
        "protected".to_owned()
    } else {
        "public".to_owned()
    }
}

fn clean_py_type_str(raw: &str) -> Option<String> {
    let mut s = raw.split(['=', ',']).next().unwrap_or(raw).trim();
    for wrapper in ["Optional[", "Union[", "Annotated["] {
        if let Some(inner) = s.strip_prefix(wrapper) {
            s = inner.trim_end_matches(']').trim();
            if wrapper == "Annotated[" {
                s = s.split(',').next().unwrap_or(s).trim();
            }
        }
    }
    let chosen = s
        .split('|')
        .map(str::trim)
        .find(|part| !part.is_empty() && *part != "None")?;
    let base = chosen.split('[').next().unwrap_or(chosen).trim();
    if matches!(
        base,
        "" | "str"
            | "int"
            | "float"
            | "bool"
            | "bytes"
            | "list"
            | "dict"
            | "set"
            | "tuple"
            | "Any"
            | "None"
            | "Self"
            | "object"
            | "Callable"
    ) {
        return None;
    }
    Some(base.to_owned())
}

fn extract_py_params(
    ctx: &mut ExtractCtx<'_>,
    params_node: Node<'_>,
    owner: Option<&str>,
    var_types: &mut BTreeMap<String, String>,
) {
    for i in 0..params_node.child_count() {
        let Some(p) = params_node.child(i) else {
            continue;
        };
        if !matches!(p.kind(), "typed_parameter" | "typed_default_parameter") {
            continue;
        }
        let pname = p
            .child(0)
            .map(|n| ctx.text(n).trim().trim_start_matches('*').to_owned())
            .unwrap_or_default();
        if let Some(ty_node) = p.child_by_field_name("type")
            && let Some(ty) = clean_py_type_str(ctx.text(ty_node))
        {
            if !pname.is_empty() && !matches!(pname.as_str(), "self" | "cls") {
                var_types.insert(pname, ty.clone());
            }
            ctx.push_ref(AstRef {
                target_name: ty,
                member: None,
                context: "parameter_type".to_owned(),
                access: "type".to_owned(),
                member_kind: None,
                owner_type: None,
                enclosing_symbol: owner.map(str::to_owned),
                is_static: false,
                line: start_line(p),
                evidence: clip_str(ctx.text(p), 160),
            });
        }
    }
}

fn infer_py_expr_type(
    ctx: &ExtractCtx<'_>,
    rhs: Node<'_>,
    this_attrs: &BTreeMap<String, String>,
    var_types: &BTreeMap<String, String>,
) -> Option<String> {
    match rhs.kind() {
        "call" => rhs
            .child_by_field_name("function")
            .map(|f| ctx.text(f).trim().to_owned())
            .filter(|s| {
                s.rsplit('.')
                    .next()
                    .is_some_and(|c| c.chars().next().is_some_and(|ch| ch.is_ascii_uppercase()))
            }),
        "attribute" => {
            let txt = ctx.text(rhs).trim();
            txt.strip_prefix("self.")
                .and_then(|a| this_attrs.get(a.trim()).cloned())
        }
        "identifier" => var_types.get(ctx.text(rhs).trim()).cloned(),
        _ => None,
    }
}

fn resolve_py_receiver_type(
    recv: &str,
    this_attrs: &BTreeMap<String, String>,
    var_types: &BTreeMap<String, String>,
) -> Option<String> {
    let trimmed = recv.trim();
    if let Some(attr) = trimmed.strip_prefix("self.") {
        return this_attrs.get(attr.trim()).cloned();
    }
    var_types.get(trimmed).cloned()
}

fn emit_py_attr(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    is_write: bool,
    owner_class: Option<&str>,
    this_attrs: &BTreeMap<String, String>,
    var_types: &BTreeMap<String, String>,
) {
    let (Some(obj), Some(attr)) = (
        node.child_by_field_name("object"),
        node.child_by_field_name("attribute"),
    ) else {
        return;
    };
    let obj_txt = ctx.text(obj).trim();
    let attr_txt = ctx.text(attr).trim();
    if obj_txt == "self" {
        return;
    }
    let owner_ty = resolve_py_receiver_type(obj_txt, this_attrs, var_types);
    let target = owner_ty.clone().unwrap_or_else(|| obj_txt.to_owned());
    let is_const = !is_write && is_upper_snake(attr_txt);
    ctx.push_ref(AstRef {
        target_name: target,
        member: Some(attr_txt.to_owned()),
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
        owner_type: owner_ty,
        enclosing_symbol: owner_class.map(str::to_owned),
        is_static: false,
        line: start_line(node),
        evidence: clip_str(ctx.text(node), 160),
    });
}

// =========================================================================
// 5. Go Extraction
// =========================================================================
