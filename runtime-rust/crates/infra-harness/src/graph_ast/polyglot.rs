use super::*;

pub(super) fn extract_go(ctx: &mut ExtractCtx<'_>, root: Node<'_>) {
    walk_go(ctx, root, 0, None, 0);
}

fn walk_go(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    depth: usize,
    owner_type: Option<&str>,
    fn_depth: usize,
) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    match node.kind() {
        "package_clause" => {
            if let Some(pkg) = child_by_kind(node, "package_identifier") {
                ctx.namespace = Some(ctx.text(pkg).trim().to_owned());
            }
            return;
        }
        "import_declaration" => {
            let line = start_line(node);
            let evidence = ctx.text(node);
            let mut stack = vec![node];
            while let Some(cur) = stack.pop() {
                if cur.kind() == "import_spec" {
                    let path = cur
                        .child_by_field_name("path")
                        .and_then(|p| unquote_literal(ctx.text(p)))
                        .unwrap_or_default();
                    let alias = cur
                        .child_by_field_name("name")
                        .map(|n| ctx.text(n).trim().to_owned());
                    let short = path.rsplit('/').next().unwrap_or(&path).to_owned();
                    ctx.push_import(AstImport {
                        path,
                        symbols: if short.is_empty() {
                            vec![]
                        } else {
                            vec![short]
                        },
                        alias,
                        is_wildcard: false,
                        is_reexport: false,
                        reexport_visibility: None,
                        kind: "import".to_owned(),
                        line: start_line(cur),
                        evidence: clip_str(evidence, 160),
                    });
                    continue;
                }
                for i in (0..cur.child_count()).rev() {
                    if let Some(ch) = cur.child(i) {
                        stack.push(ch);
                    }
                }
            }
            let _ = line;
            return;
        }
        "type_spec" => {
            if fn_depth > 0 {
                return;
            }
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let ty_node = node.child_by_field_name("type");
            let decl_kind = match ty_node.map(|t| t.kind()) {
                Some("struct_type") => "struct",
                Some("interface_type") => "interface",
                _ => "type",
            };
            let line = start_line(node);
            let e_line = end_line(node);
            let sig = header_without_comments(
                node,
                ty_node.map_or(node.end_byte(), |t| t.start_byte()),
                ctx.source,
            );
            let vis = go_visibility(&name);
            if !name.is_empty() {
                ctx.push_decl(
                    decl_kind,
                    &name,
                    Some(ctx.rel_path.clone()),
                    line,
                    e_line,
                    Some(vis),
                    Vec::new(),
                    format!("type {sig} {decl_kind}"),
                    format!("type {sig} {decl_kind}"),
                    String::new(),
                );
                if let Some(t) = ty_node {
                    walk_go(ctx, t, depth + 1, Some(&name), fn_depth);
                }
            }
            return;
        }
        "field_declaration" => {
            if let Some(owner) = owner_type
                && let Some(name_node) = node.child_by_field_name("name")
            {
                let name = ctx.text(name_node).trim().to_owned();
                let sig = header_without_comments(node, node.end_byte(), ctx.source);
                ctx.push_decl(
                    "field",
                    &name,
                    Some(owner.to_owned()),
                    start_line(node),
                    end_line(node),
                    Some(go_visibility(&name)),
                    Vec::new(),
                    sig.clone(),
                    sig,
                    String::new(),
                );
            }
            return;
        }
        "method_elem" => {
            if let Some(owner) = owner_type
                && let Some(name_node) = node.child_by_field_name("name")
            {
                let name = ctx.text(name_node).trim().to_owned();
                let sig = header_without_comments(node, node.end_byte(), ctx.source);
                ctx.push_decl(
                    "method",
                    &name,
                    Some(owner.to_owned()),
                    start_line(node),
                    end_line(node),
                    Some(go_visibility(&name)),
                    Vec::new(),
                    sig.clone(),
                    sig,
                    String::new(),
                );
            }
            return;
        }
        "const_spec" => {
            if fn_depth == 0
                && let Some(name_node) = node.child_by_field_name("name")
            {
                let name = ctx.text(name_node).trim().to_owned();
                let val_node = node.child_by_field_name("value");
                let sig_end = val_node.map_or(node.end_byte(), |v| v.start_byte());
                let sig = header_without_comments(node, sig_end, ctx.source)
                    .trim_end_matches('=')
                    .trim()
                    .to_owned();
                ctx.push_decl(
                    "const",
                    &name,
                    Some(ctx.rel_path.clone()),
                    start_line(node),
                    end_line(node),
                    Some(go_visibility(&name)),
                    Vec::new(),
                    sig.clone(),
                    sig,
                    val_node
                        .map(|v| ast_body_fingerprint(v, ctx.source))
                        .unwrap_or_default(),
                );
            }
            return;
        }
        "function_declaration" | "method_declaration" => {
            if fn_depth > 0 {
                return;
            }
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let recv_owner = node.child_by_field_name("receiver").and_then(|r| {
                let raw = ctx.text(r);
                raw.trim_matches(|c| matches!(c, '(' | ')' | ' '))
                    .split_whitespace()
                    .last()
                    .map(|t| t.trim_start_matches('*').to_owned())
            });
            let body_node = node.child_by_field_name("body");
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source);
            let body_fp = body_node
                .map(|b| ast_body_fingerprint(b, ctx.source))
                .unwrap_or_default();
            let decl_kind = if recv_owner.is_some() {
                "method"
            } else {
                "function"
            };
            if !name.is_empty() {
                ctx.push_decl(
                    decl_kind,
                    &name,
                    recv_owner.clone(),
                    start_line(node),
                    end_line(node),
                    Some(go_visibility(&name)),
                    Vec::new(),
                    sig.clone(),
                    sig,
                    body_fp,
                );
            }
            if let Some(body) = body_node {
                walk_go(ctx, body, depth + 1, recv_owner.as_deref(), fn_depth + 1);
            }
            return;
        }
        "call_expression" => {
            if let Some(fn_node) = node.child_by_field_name("function") {
                let line = start_line(node);
                let evidence = ctx.text(node);
                let args_node = node.child_by_field_name("arguments");
                let args_txt = args_node.map(|a| ctx.text(a)).unwrap_or("");
                let str_args = args_node
                    .map(|a| all_string_arg_texts(a, ctx.source))
                    .unwrap_or_default();
                if fn_node.kind() == "selector_expression" {
                    if let (Some(op), Some(field)) = (
                        fn_node.child_by_field_name("operand"),
                        fn_node.child_by_field_name("field"),
                    ) {
                        let op_txt = ctx.text(op).trim();
                        let field_txt = ctx.text(field).trim();
                        emit_semantic_from_call(
                            ctx,
                            Some(op_txt),
                            field_txt,
                            args_txt,
                            &str_args,
                            evidence,
                            owner_type.map(str::to_owned),
                            line,
                        );
                        ctx.push_ref(AstRef {
                            target_name: op_txt.to_owned(),
                            member: Some(field_txt.to_owned()),
                            context: "call".to_owned(),
                            access: "call".to_owned(),
                            member_kind: Some("function".to_owned()),
                            owner_type: None,
                            enclosing_symbol: owner_type.map(str::to_owned),
                            is_static: false,
                            line,
                            evidence: clip_str(evidence, 160),
                        });
                    }
                } else {
                    let fn_txt = ctx.text(fn_node).trim();
                    emit_semantic_from_call(
                        ctx,
                        None,
                        fn_txt,
                        args_txt,
                        &str_args,
                        evidence,
                        owner_type.map(str::to_owned),
                        line,
                    );
                    ctx.push_ref(AstRef {
                        target_name: fn_txt.to_owned(),
                        member: None,
                        context: "call".to_owned(),
                        access: "call".to_owned(),
                        member_kind: Some("function".to_owned()),
                        owner_type: None,
                        enclosing_symbol: owner_type.map(str::to_owned),
                        is_static: false,
                        line,
                        evidence: clip_str(evidence, 160),
                    });
                }
            }
        }
        "selector_expression" => {
            if node.parent().is_some_and(|p| p.kind() != "call_expression")
                && let (Some(op), Some(field)) = (
                    node.child_by_field_name("operand"),
                    node.child_by_field_name("field"),
                )
            {
                let op_txt = ctx.text(op).trim();
                let field_txt = ctx.text(field).trim();
                ctx.push_ref(AstRef {
                    target_name: op_txt.to_owned(),
                    member: Some(field_txt.to_owned()),
                    context: "field".to_owned(),
                    access: "read".to_owned(),
                    member_kind: Some("field".to_owned()),
                    owner_type: None,
                    enclosing_symbol: owner_type.map(str::to_owned),
                    is_static: false,
                    line: start_line(node),
                    evidence: clip_str(ctx.text(node), 160),
                });
            }
        }
        _ => {}
    }
    for i in 0..node.child_count() {
        if let Some(ch) = node.child(i) {
            walk_go(ctx, ch, depth + 1, owner_type, fn_depth);
        }
    }
}

fn go_visibility(name: &str) -> String {
    if name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
        "public".to_owned()
    } else {
        "package".to_owned()
    }
}

// =========================================================================
// 6. Java Extraction
// =========================================================================

pub(super) fn extract_java(ctx: &mut ExtractCtx<'_>, root: Node<'_>) {
    walk_java(ctx, root, 0, None, 0);
}

fn walk_java(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    depth: usize,
    owner_class: Option<&str>,
    fn_depth: usize,
) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    match node.kind() {
        "package_declaration" => {
            let pkg = ctx
                .text(node)
                .trim_start_matches("package")
                .trim_end_matches(';')
                .trim()
                .to_owned();
            if !pkg.is_empty() {
                ctx.namespace = Some(pkg);
            }
            return;
        }
        "import_declaration" => {
            let raw = ctx
                .text(node)
                .trim_start_matches("import")
                .trim()
                .trim_start_matches("static")
                .trim()
                .trim_end_matches(';')
                .trim();
            let is_wc = raw.ends_with(".*");
            let path = raw.trim_end_matches(".*").to_owned();
            let sym = path.rsplit('.').next().unwrap_or(&path).to_owned();
            ctx.push_import(AstImport {
                path,
                symbols: if is_wc || sym.is_empty() {
                    Vec::new()
                } else {
                    vec![sym]
                },
                alias: None,
                is_wildcard: is_wc,
                is_reexport: false,
                reexport_visibility: None,
                kind: "import".to_owned(),
                line: start_line(node),
                evidence: clip_str(ctx.text(node), 160),
            });
            return;
        }
        "class_declaration"
        | "interface_declaration"
        | "enum_declaration"
        | "record_declaration"
        | "annotation_type_declaration" => {
            let decl_kind = match node.kind() {
                "interface_declaration" | "annotation_type_declaration" => "interface",
                "enum_declaration" => "enum",
                _ => "class",
            };
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let line = start_line(node);
            let body_node = node.child_by_field_name("body");
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source);
            let (vis, mods) = parse_jvm_modifiers(node, ctx.source, "package");
            emit_jvm_annotations(ctx, node, Some(&name), &[]);
            if !name.is_empty() && fn_depth == 0 {
                ctx.push_decl(
                    decl_kind,
                    &name,
                    owner_class.map(str::to_owned),
                    line,
                    end_line(node),
                    Some(vis),
                    mods,
                    sig.clone(),
                    strip_leading_vis_word(&sig),
                    String::new(),
                );
                if let Some(sc) = node.child_by_field_name("superclass") {
                    let base = ctx
                        .text(sc)
                        .trim_start_matches("extends")
                        .split('<')
                        .next()
                        .unwrap_or("")
                        .trim();
                    ctx.push_inheritance(&name, base, "extends", line, ctx.text(sc));
                }
                if let Some(ifaces) = node.child_by_field_name("interfaces") {
                    for part in ctx.text(ifaces).trim_start_matches("implements").split(',') {
                        let p = part.split('<').next().unwrap_or("").trim();
                        ctx.push_inheritance(&name, p, "implements", line, ctx.text(ifaces));
                    }
                }
                if let Some(ext_ifaces) = child_by_kind(node, "extends_interfaces") {
                    for part in ctx
                        .text(ext_ifaces)
                        .trim_start_matches("extends")
                        .split(',')
                    {
                        let p = part.split('<').next().unwrap_or("").trim();
                        ctx.push_inheritance(&name, p, "extends", line, ctx.text(ext_ifaces));
                    }
                }
                if let Some(body) = body_node {
                    walk_java(ctx, body, depth + 1, Some(&name), fn_depth);
                }
            }
            return;
        }
        "method_declaration" | "constructor_declaration" => {
            if let Some(owner) = owner_class {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| ctx.text(n).trim().to_owned())
                    .unwrap_or_default();
                let body_node = node.child_by_field_name("body");
                let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
                let sig = header_without_comments(node, sig_end, ctx.source)
                    .trim_end_matches(';')
                    .trim()
                    .to_owned();
                let (vis, mods) = parse_jvm_modifiers(node, ctx.source, "package");
                let param_types = node
                    .child_by_field_name("parameters")
                    .map(|p| collect_java_param_types(p, ctx.source))
                    .unwrap_or_default();
                emit_jvm_annotations(ctx, node, Some(&format!("{owner}::{name}")), &param_types);
                ctx.push_decl(
                    "method",
                    &name,
                    Some(owner.to_owned()),
                    start_line(node),
                    end_line(node),
                    Some(vis),
                    mods,
                    sig.clone(),
                    strip_leading_vis_word(&sig),
                    body_node
                        .map(|b| ast_body_fingerprint(b, ctx.source))
                        .unwrap_or_default(),
                );
                if let Some(body) = body_node {
                    walk_java(ctx, body, depth + 1, Some(owner), fn_depth + 1);
                }
                return;
            }
        }
        "field_declaration" | "constant_declaration" => {
            if let Some(owner) = owner_class {
                let (vis, mods) = parse_jvm_modifiers(node, ctx.source, "package");
                let is_const = node.kind() == "constant_declaration"
                    || (mods.iter().any(|m| m == "static") && mods.iter().any(|m| m == "final"));
                for decl in children_by_kind(node, "variable_declarator") {
                    if let Some(n) = decl.child_by_field_name("name") {
                        let name = ctx.text(n).trim().to_owned();
                        let val_node = decl.child_by_field_name("value");
                        let sig = header_without_comments(
                            node,
                            val_node.map_or(decl.end_byte(), |v| v.start_byte()),
                            ctx.source,
                        )
                        .trim_end_matches(['=', ';'])
                        .trim()
                        .to_owned();
                        ctx.push_decl(
                            if is_const { "const" } else { "field" },
                            &name,
                            Some(owner.to_owned()),
                            start_line(decl),
                            end_line(decl),
                            Some(vis.clone()),
                            mods.clone(),
                            sig.clone(),
                            strip_leading_vis_word(&sig),
                            val_node
                                .map(|v| ast_body_fingerprint(v, ctx.source))
                                .unwrap_or_default(),
                        );
                    }
                }
                return;
            }
        }
        "object_creation_expression" => {
            if let Some(ty) = node.child_by_field_name("type") {
                let name = ctx
                    .text(ty)
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
        "method_invocation" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let mem = ctx.text(name_node).trim().to_owned();
                let obj = node
                    .child_by_field_name("object")
                    .map(|o| ctx.text(o).trim().to_owned());
                let args_node = node.child_by_field_name("arguments");
                let args_txt = args_node.map(|a| ctx.text(a)).unwrap_or("");
                let str_args = args_node
                    .map(|a| all_string_arg_texts(a, ctx.source))
                    .unwrap_or_default();
                let line = start_line(node);
                let evidence = ctx.text(node);
                emit_semantic_from_call(
                    ctx,
                    obj.as_deref(),
                    &mem,
                    args_txt,
                    &str_args,
                    evidence,
                    owner_class.map(str::to_owned),
                    line,
                );
                let target = obj.clone().unwrap_or_else(|| mem.clone());
                ctx.push_ref(AstRef {
                    target_name: target,
                    member: obj.is_some().then_some(mem),
                    context: "call".to_owned(),
                    access: "call".to_owned(),
                    member_kind: Some(if obj.is_some() { "method" } else { "function" }.to_owned()),
                    owner_type: obj,
                    enclosing_symbol: owner_class.map(str::to_owned),
                    is_static: false,
                    line,
                    evidence: clip_str(evidence, 160),
                });
            }
        }
        _ => {}
    }
    for i in 0..node.child_count() {
        if let Some(ch) = node.child(i) {
            walk_java(ctx, ch, depth + 1, owner_class, fn_depth);
        }
    }
}

fn parse_jvm_modifiers(node: Node<'_>, source: &str, default_vis: &str) -> (String, Vec<String>) {
    let mut vis = default_vis.to_owned();
    let mut mods = Vec::new();
    if let Some(mnode) = child_by_kind(node, "modifiers") {
        for word in node_text(mnode, source).split_whitespace() {
            if matches!(word, "public" | "protected" | "private" | "internal") {
                vis = word.to_owned();
            } else if !word.starts_with('@') {
                push_unique_mod(&mut mods, word);
            }
        }
    }
    (vis, mods)
}

fn emit_jvm_annotations(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    sym: Option<&str>,
    param_types: &[String],
) {
    let mut stack = vec![node];
    while let Some(cur) = stack.pop() {
        if matches!(cur.kind(), "annotation" | "marker_annotation") {
            let txt = ctx.text(cur);
            let name = cur
                .child_by_field_name("name")
                .map(|n| ctx.text(n))
                .unwrap_or(txt);
            let strs = all_string_arg_texts(cur, ctx.source);
            emit_semantic_from_attribute(ctx, name, txt, &strs, sym, param_types, start_line(cur));
            continue;
        }
        // Only descend into `modifiers` / direct annotation siblings, not body!
        if cur == node || cur.kind() == "modifiers" {
            for i in (0..cur.child_count()).rev() {
                if let Some(ch) = cur.child(i) {
                    stack.push(ch);
                }
            }
        }
    }
}

fn collect_java_param_types(params: Node<'_>, source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for p in children_by_kind(params, "formal_parameter") {
        if let Some(ty) = p.child_by_field_name("type") {
            let short = node_text(ty, source)
                .split('<')
                .next()
                .unwrap_or("")
                .rsplit('.')
                .next()
                .unwrap_or("")
                .trim();
            if !short.is_empty() {
                out.push(short.to_owned());
            }
        }
    }
    out
}

fn strip_leading_vis_word(sig: &str) -> String {
    let mut parts = Vec::new();
    let mut skip = true;
    for tok in sig.split_whitespace() {
        if skip && matches!(tok, "public" | "protected" | "private" | "internal") {
            continue;
        }
        skip = false;
        parts.push(tok);
    }
    parts.join(" ")
}

// =========================================================================
// 7. C# Extraction
// =========================================================================

pub(super) fn extract_csharp(ctx: &mut ExtractCtx<'_>, root: Node<'_>) {
    walk_csharp(ctx, root, 0, None, 0);
}

fn walk_csharp(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    depth: usize,
    owner_class: Option<&str>,
    fn_depth: usize,
) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    match node.kind() {
        "namespace_declaration" | "file_scoped_namespace_declaration" => {
            if let Some(name) = node.child_by_field_name("name") {
                ctx.namespace = Some(ctx.text(name).trim().to_owned());
            }
        }
        "using_directive" => {
            let raw = ctx
                .text(node)
                .trim_start_matches("global")
                .trim()
                .trim_start_matches("using")
                .trim()
                .trim_start_matches("static")
                .trim()
                .trim_end_matches(';')
                .trim();
            let (alias, path) = if let Some((a, p)) = raw.split_once('=') {
                (Some(a.trim().to_owned()), p.trim().to_owned())
            } else {
                (None, raw.to_owned())
            };
            let sym = path.rsplit('.').next().unwrap_or(&path).to_owned();
            let is_wildcard = alias.is_none();
            ctx.push_import(AstImport {
                path,
                symbols: if sym.is_empty() { vec![] } else { vec![sym] },
                alias,
                is_wildcard,
                is_reexport: false,
                reexport_visibility: None,
                kind: "import".to_owned(),
                line: start_line(node),
                evidence: clip_str(ctx.text(node), 160),
            });
            return;
        }
        "class_declaration"
        | "struct_declaration"
        | "interface_declaration"
        | "enum_declaration"
        | "record_declaration" => {
            let decl_kind = match node.kind() {
                "struct_declaration" => "struct",
                "interface_declaration" => "interface",
                "enum_declaration" => "enum",
                _ => "class",
            };
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let line = start_line(node);
            let body_node = node.child_by_field_name("body");
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source);
            let (vis, mods) = parse_cs_modifiers(node, ctx.source, "internal");
            emit_cs_attributes(ctx, node, Some(&name));
            if !name.is_empty() && fn_depth == 0 {
                ctx.push_decl(
                    decl_kind,
                    &name,
                    owner_class.map(str::to_owned),
                    line,
                    end_line(node),
                    Some(vis),
                    mods,
                    sig.clone(),
                    strip_leading_vis_word(&sig),
                    String::new(),
                );
                if let Some(base_list) = child_by_kind(node, "base_list") {
                    for (idx, part) in ctx
                        .text(base_list)
                        .trim_start_matches(':')
                        .split(',')
                        .enumerate()
                    {
                        let base = part.split(['<', '(']).next().unwrap_or("").trim();
                        let ik = if idx == 0 && !base.starts_with('I') {
                            "extends"
                        } else {
                            "implements"
                        };
                        ctx.push_inheritance(&name, base, ik, line, ctx.text(base_list));
                    }
                }
                if let Some(body) = body_node {
                    walk_csharp(ctx, body, depth + 1, Some(&name), fn_depth);
                }
            }
            return;
        }
        "method_declaration" | "constructor_declaration" => {
            if let Some(owner) = owner_class {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| ctx.text(n).trim().to_owned())
                    .unwrap_or_default();
                let body_node = node.child_by_field_name("body");
                let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
                let sig = header_without_comments(node, sig_end, ctx.source)
                    .trim_end_matches(';')
                    .trim()
                    .to_owned();
                let (vis, mods) = parse_cs_modifiers(node, ctx.source, "private");
                emit_cs_attributes(ctx, node, Some(&format!("{owner}::{name}")));
                ctx.push_decl(
                    "method",
                    &name,
                    Some(owner.to_owned()),
                    start_line(node),
                    end_line(node),
                    Some(vis),
                    mods,
                    sig.clone(),
                    strip_leading_vis_word(&sig),
                    body_node
                        .map(|b| ast_body_fingerprint(b, ctx.source))
                        .unwrap_or_default(),
                );
                if let Some(body) = body_node {
                    walk_csharp(ctx, body, depth + 1, Some(owner), fn_depth + 1);
                }
                return;
            }
        }
        "property_declaration" => {
            if let Some(owner) = owner_class
                && let Some(n) = node.child_by_field_name("name")
            {
                let name = ctx.text(n).trim().to_owned();
                let (vis, mods) = parse_cs_modifiers(node, ctx.source, "private");
                let sig = header_without_comments(
                    node,
                    node.child_by_field_name("accessors")
                        .map_or(node.end_byte(), |a| a.start_byte()),
                    ctx.source,
                );
                ctx.push_decl(
                    "property",
                    &name,
                    Some(owner.to_owned()),
                    start_line(node),
                    end_line(node),
                    Some(vis),
                    mods,
                    sig.clone(),
                    strip_leading_vis_word(&sig),
                    String::new(),
                );
            }
            return;
        }
        "field_declaration" => {
            if let Some(owner) = owner_class
                && let Some(vd) = child_by_kind(node, "variable_declaration")
            {
                let (vis, mods) = parse_cs_modifiers(node, ctx.source, "private");
                let is_const = mods.iter().any(|m| m == "const");
                for decl in children_by_kind(vd, "variable_declarator") {
                    if let Some(n) = decl.child_by_field_name("name") {
                        let name = ctx.text(n).trim().to_owned();
                        let sig = header_without_comments(node, decl.end_byte(), ctx.source)
                            .split('=')
                            .next()
                            .unwrap_or("")
                            .trim()
                            .to_owned();
                        ctx.push_decl(
                            if is_const { "const" } else { "field" },
                            &name,
                            Some(owner.to_owned()),
                            start_line(decl),
                            end_line(decl),
                            Some(vis.clone()),
                            mods.clone(),
                            sig.clone(),
                            strip_leading_vis_word(&sig),
                            String::new(),
                        );
                    }
                }
                return;
            }
        }
        "object_creation_expression" => {
            if let Some(ty) = node.child_by_field_name("type") {
                ctx.push_ref(AstRef {
                    target_name: ctx
                        .text(ty)
                        .split('<')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_owned(),
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
        "invocation_expression" => {
            if let Some(fn_node) = node.child_by_field_name("function") {
                let line = start_line(node);
                let evidence = ctx.text(node);
                let args_node = node.child_by_field_name("arguments");
                let args_txt = args_node.map(|a| ctx.text(a)).unwrap_or("");
                let str_args = args_node
                    .map(|a| all_string_arg_texts(a, ctx.source))
                    .unwrap_or_default();
                if fn_node.kind() == "member_access_expression" {
                    let obj = fn_node
                        .child_by_field_name("expression")
                        .map(|n| ctx.text(n).trim().to_owned())
                        .unwrap_or_default();
                    let mem = fn_node
                        .child_by_field_name("name")
                        .map(|n| ctx.text(n).trim().to_owned())
                        .unwrap_or_default();
                    let mem_clean = mem.split('<').next().unwrap_or(&mem);
                    emit_semantic_from_call(
                        ctx,
                        Some(&obj),
                        mem_clean,
                        args_txt,
                        &str_args,
                        evidence,
                        owner_class.map(str::to_owned),
                        line,
                    );
                    ctx.push_ref(AstRef {
                        target_name: obj.clone(),
                        member: Some(mem_clean.to_owned()),
                        context: "call".to_owned(),
                        access: "call".to_owned(),
                        member_kind: Some("method".to_owned()),
                        owner_type: Some(obj),
                        enclosing_symbol: owner_class.map(str::to_owned),
                        is_static: false,
                        line,
                        evidence: clip_str(evidence, 160),
                    });
                } else {
                    let fn_txt = ctx.text(fn_node).trim();
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
                }
            }
        }
        _ => {}
    }
    for i in 0..node.child_count() {
        if let Some(ch) = node.child(i) {
            walk_csharp(ctx, ch, depth + 1, owner_class, fn_depth);
        }
    }
}

fn parse_cs_modifiers(node: Node<'_>, source: &str, default_vis: &str) -> (String, Vec<String>) {
    let mut vis = default_vis.to_owned();
    let mut mods = Vec::new();
    for m in children_by_kind(node, "modifier") {
        let txt = node_text(m, source).trim();
        if matches!(txt, "public" | "private" | "protected" | "internal") {
            vis = txt.to_owned();
        } else {
            push_unique_mod(&mut mods, txt);
        }
    }
    (vis, mods)
}

fn emit_cs_attributes(ctx: &mut ExtractCtx<'_>, node: Node<'_>, sym: Option<&str>) {
    for al in children_by_kind(node, "attribute_list") {
        for attr in children_by_kind(al, "attribute") {
            let txt = ctx.text(attr);
            let name = attr
                .child_by_field_name("name")
                .map(|n| ctx.text(n))
                .unwrap_or(txt);
            let strs = all_string_arg_texts(attr, ctx.source);
            emit_semantic_from_attribute(ctx, name, txt, &strs, sym, &[], start_line(attr));
        }
    }
}

// =========================================================================
// 8. Ruby Extraction
// =========================================================================

pub(super) fn extract_ruby(ctx: &mut ExtractCtx<'_>, root: Node<'_>) {
    let mut vis = "public".to_owned();
    walk_ruby(ctx, root, 0, None, 0, &mut vis);
}

fn walk_ruby(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    depth: usize,
    owner_class: Option<&str>,
    fn_depth: usize,
    cur_vis: &mut String,
) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    match node.kind() {
        "class" | "module" => {
            let decl_kind = node.kind();
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let line = start_line(node);
            let body_node = node.child_by_field_name("body");
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source);
            if !name.is_empty() {
                ctx.push_decl(
                    decl_kind,
                    &name,
                    owner_class.map(str::to_owned),
                    line,
                    end_line(node),
                    Some("public".to_owned()),
                    Vec::new(),
                    sig.clone(),
                    sig,
                    String::new(),
                );
                if let Some(sc) = node.child_by_field_name("superclass") {
                    let base = ctx.text(sc).trim_start_matches('<').trim();
                    ctx.push_inheritance(&name, base, "extends", line, ctx.text(sc));
                }
                if let Some(body) = body_node {
                    let mut class_vis = "public".to_owned();
                    walk_ruby(ctx, body, depth + 1, Some(&name), fn_depth, &mut class_vis);
                }
            }
            return;
        }
        "method" | "singleton_method" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let body_node = node.child_by_field_name("body");
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source);
            let mut mods = Vec::new();
            if node.kind() == "singleton_method" {
                mods.push("static".to_owned());
            }
            let decl_kind = if owner_class.is_some() {
                "method"
            } else {
                "function"
            };
            if !name.is_empty() {
                ctx.push_decl(
                    decl_kind,
                    &name,
                    owner_class.map(str::to_owned),
                    start_line(node),
                    end_line(node),
                    Some(cur_vis.clone()),
                    mods,
                    sig.clone(),
                    sig,
                    body_node
                        .map(|b| ast_body_fingerprint(b, ctx.source))
                        .unwrap_or_default(),
                );
            }
            if let Some(body) = body_node {
                let mut inner_vis = cur_vis.clone();
                walk_ruby(
                    ctx,
                    body,
                    depth + 1,
                    owner_class,
                    fn_depth + 1,
                    &mut inner_vis,
                );
            }
            return;
        }
        "identifier" if owner_class.is_some() && fn_depth == 0 => {
            let txt = ctx.text(node).trim();
            if matches!(txt, "private" | "protected" | "public") {
                *cur_vis = txt.to_owned();
            }
        }
        "assignment" => {
            if let (Some(lhs), Some(rhs)) = (
                node.child_by_field_name("left"),
                node.child_by_field_name("right"),
            ) {
                let lhs_txt = ctx.text(lhs).trim();
                if fn_depth == 0 && lhs.kind() == "constant" {
                    ctx.push_decl(
                        "const",
                        lhs_txt,
                        owner_class.map(str::to_owned),
                        start_line(node),
                        end_line(node),
                        Some("public".to_owned()),
                        Vec::new(),
                        lhs_txt.to_owned(),
                        lhs_txt.to_owned(),
                        ast_body_fingerprint(rhs, ctx.source),
                    );
                } else if lhs_txt == "self.table_name"
                    && let Some(tbl) =
                        unquote_literal(ctx.text(rhs)).and_then(|s| clean_sql_table_name(&s))
                {
                    ctx.push_semantic(
                        "reads_table",
                        format!("table:{tbl}"),
                        owner_class.map(str::to_owned),
                        start_line(node),
                        ctx.text(node),
                    );
                }
            }
        }
        "call" => {
            let method_name = node
                .child_by_field_name("method")
                .map(|m| ctx.text(m).trim().to_owned())
                .unwrap_or_default();
            let recv = node
                .child_by_field_name("receiver")
                .map(|r| ctx.text(r).trim().to_owned());
            let args_node = node.child_by_field_name("arguments");
            let args_txt = args_node.map(|a| ctx.text(a)).unwrap_or("");
            let str_args = args_node
                .map(|a| all_string_arg_texts(a, ctx.source))
                .unwrap_or_default();
            let line = start_line(node);
            let evidence = ctx.text(node);

            if matches!(method_name.as_str(), "require" | "require_relative")
                && let Some(path) = str_args.first()
            {
                let stem = path.rsplit('/').next().unwrap_or(path).to_owned();
                ctx.push_import(AstImport {
                    path: path.clone(),
                    symbols: vec![stem],
                    alias: None,
                    is_wildcard: false,
                    is_reexport: false,
                    reexport_visibility: None,
                    kind: if method_name == "require_relative" {
                        "include"
                    } else {
                        "import"
                    }
                    .to_owned(),
                    line,
                    evidence: clip_str(evidence, 160),
                });
            } else if matches!(method_name.as_str(), "include" | "extend" | "prepend")
                && let Some(owner) = owner_class
                && let Some(args) = args_node
            {
                for part in ctx.text(args).trim_matches(['(', ')']).split(',') {
                    let mixin = part.trim();
                    if !mixin.is_empty() {
                        ctx.push_inheritance(owner, mixin, "uses", line, evidence);
                    }
                }
            } else {
                emit_semantic_from_call(
                    ctx,
                    recv.as_deref(),
                    &method_name,
                    args_txt,
                    &str_args,
                    evidence,
                    owner_class.map(str::to_owned),
                    line,
                );
                let target = recv.clone().unwrap_or_else(|| method_name.clone());
                ctx.push_ref(AstRef {
                    target_name: target,
                    member: recv.is_some().then_some(method_name),
                    context: "call".to_owned(),
                    access: "call".to_owned(),
                    member_kind: Some(
                        if recv.is_some() { "method" } else { "function" }.to_owned(),
                    ),
                    owner_type: recv,
                    enclosing_symbol: owner_class.map(str::to_owned),
                    is_static: false,
                    line,
                    evidence: clip_str(evidence, 160),
                });
            }
        }
        _ => {}
    }
    for i in 0..node.child_count() {
        if let Some(ch) = node.child(i) {
            walk_ruby(ctx, ch, depth + 1, owner_class, fn_depth, cur_vis);
        }
    }
}

// =========================================================================
// 9. Kotlin Extraction
// =========================================================================

pub(super) fn extract_kotlin(ctx: &mut ExtractCtx<'_>, root: Node<'_>) {
    walk_kotlin(ctx, root, 0, None, 0);
}

fn walk_kotlin(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    depth: usize,
    owner_class: Option<&str>,
    fn_depth: usize,
) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    match node.kind() {
        "package_header" => {
            let pkg = ctx
                .text(node)
                .trim_start_matches("package")
                .trim()
                .to_owned();
            if !pkg.is_empty() {
                ctx.namespace = Some(pkg);
            }
            return;
        }
        "import" => {
            let raw = ctx.text(node).trim_start_matches("import").trim();
            let (path_part, alias) = if let Some((l, r)) = raw.split_once(" as ") {
                (l.trim(), Some(r.trim().to_owned()))
            } else {
                (raw, None)
            };
            let is_wc = path_part.ends_with(".*");
            let clean = path_part.trim_end_matches(".*").to_owned();
            let sym = clean.rsplit('.').next().unwrap_or(&clean).to_owned();
            ctx.push_import(AstImport {
                path: clean,
                symbols: if is_wc || sym.is_empty() {
                    vec![]
                } else {
                    vec![sym]
                },
                alias,
                is_wildcard: is_wc,
                is_reexport: false,
                reexport_visibility: None,
                kind: "import".to_owned(),
                line: start_line(node),
                evidence: clip_str(ctx.text(node), 160),
            });
            return;
        }
        "class_declaration" | "object_declaration" => {
            let raw_txt = ctx.text(node);
            let decl_kind = if raw_txt.contains("interface ") {
                "interface"
            } else if raw_txt.contains("enum ") {
                "enum"
            } else {
                "class"
            };
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let body_node = child_by_kind(node, "class_body")
                .or_else(|| child_by_kind(node, "enum_class_body"));
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source);
            let (vis, mods) = parse_jvm_modifiers(node, ctx.source, "public");
            emit_kotlin_annotations(ctx, node, Some(&name));
            let line = start_line(node);
            if !name.is_empty() {
                ctx.push_decl(
                    decl_kind,
                    &name,
                    owner_class.map(str::to_owned),
                    line,
                    end_line(node),
                    Some(vis),
                    mods,
                    sig.clone(),
                    strip_leading_vis_word(&sig),
                    String::new(),
                );
                if let Some(deleg) = child_by_kind(node, "delegation_specifiers") {
                    for part in ctx.text(deleg).split(',') {
                        let clean = part.split(['(', '<']).next().unwrap_or("").trim();
                        let ik = if part.contains('(') {
                            "extends"
                        } else {
                            "implements"
                        };
                        ctx.push_inheritance(&name, clean, ik, line, ctx.text(deleg));
                    }
                }
                if let Some(pctor) = child_by_kind(node, "primary_constructor")
                    && let Some(cparams) = child_by_kind(pctor, "class_parameters")
                {
                    for cp in children_by_kind(cparams, "class_parameter") {
                        let cp_txt = ctx.text(cp);
                        if (cp_txt.contains("val ") || cp_txt.contains("var "))
                            && let Some(id) = child_by_kind(cp, "identifier")
                        {
                            let pname = ctx.text(id).trim().to_owned();
                            let (pvis, pmods) = parse_jvm_modifiers(cp, ctx.source, "public");
                            ctx.push_decl(
                                "property",
                                &pname,
                                Some(name.clone()),
                                start_line(cp),
                                end_line(cp),
                                Some(pvis),
                                pmods,
                                cp_txt.trim().to_owned(),
                                strip_leading_vis_word(cp_txt.trim()),
                                String::new(),
                            );
                        }
                    }
                }
                if let Some(body) = body_node {
                    walk_kotlin(ctx, body, depth + 1, Some(&name), fn_depth);
                }
            }
            return;
        }
        "function_declaration" => {
            if fn_depth > 0 {
                return;
            }
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let body_node = child_by_kind(node, "function_body");
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source);
            let (vis, mods) = parse_jvm_modifiers(node, ctx.source, "public");
            emit_kotlin_annotations(ctx, node, owner_class);
            if !name.is_empty() {
                ctx.push_decl(
                    if owner_class.is_some() {
                        "method"
                    } else {
                        "function"
                    },
                    &name,
                    owner_class.map(str::to_owned),
                    start_line(node),
                    end_line(node),
                    Some(vis),
                    mods,
                    sig.clone(),
                    strip_leading_vis_word(&sig),
                    body_node
                        .map(|b| ast_body_fingerprint(b, ctx.source))
                        .unwrap_or_default(),
                );
            }
            if let Some(body) = body_node {
                walk_kotlin(ctx, body, depth + 1, owner_class, fn_depth + 1);
            }
            return;
        }
        "property_declaration" => {
            if fn_depth == 0
                && let Some(vd) = child_by_kind(node, "variable_declaration")
                && let Some(id) = child_by_kind(vd, "identifier")
            {
                let name = ctx.text(id).trim().to_owned();
                let (vis, mods) = parse_jvm_modifiers(node, ctx.source, "public");
                let is_const = mods.iter().any(|m| m == "const");
                let sig = header_without_comments(node, node.end_byte(), ctx.source)
                    .split('=')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_owned();
                ctx.push_decl(
                    if is_const { "const" } else { "property" },
                    &name,
                    owner_class.map(str::to_owned),
                    start_line(node),
                    end_line(node),
                    Some(vis),
                    mods,
                    sig.clone(),
                    strip_leading_vis_word(&sig),
                    String::new(),
                );
            }
        }
        "call_expression" => {
            let full = ctx.text(node);
            let line = start_line(node);
            let callee_part = full.split('(').next().unwrap_or("").trim();
            let (recv, mem) = match callee_part.rsplit_once('.') {
                Some((r, m)) => (Some(r.trim()), m.trim()),
                None => (None, callee_part),
            };
            let str_args = all_string_arg_texts(node, ctx.source);
            let args_txt = full.split_once('(').map_or("", |(_, r)| r);
            emit_semantic_from_call(
                ctx,
                recv,
                mem,
                args_txt,
                &str_args,
                full,
                owner_class.map(str::to_owned),
                line,
            );
            ctx.push_ref(AstRef {
                target_name: recv.unwrap_or(mem).to_owned(),
                member: recv.is_some().then(|| mem.to_owned()),
                context: "call".to_owned(),
                access: "call".to_owned(),
                member_kind: Some(if recv.is_some() { "method" } else { "function" }.to_owned()),
                owner_type: recv.map(str::to_owned),
                enclosing_symbol: owner_class.map(str::to_owned),
                is_static: false,
                line,
                evidence: clip_str(full, 160),
            });
        }
        _ => {}
    }
    for i in 0..node.child_count() {
        if let Some(ch) = node.child(i) {
            walk_kotlin(ctx, ch, depth + 1, owner_class, fn_depth);
        }
    }
}

fn emit_kotlin_annotations(ctx: &mut ExtractCtx<'_>, node: Node<'_>, sym: Option<&str>) {
    if let Some(mnode) = child_by_kind(node, "modifiers") {
        for ann in children_by_kind(mnode, "annotation") {
            let txt = ctx.text(ann);
            let name = txt
                .trim_start_matches('@')
                .split('(')
                .next()
                .unwrap_or("")
                .trim();
            let strs = all_string_arg_texts(ann, ctx.source);
            emit_semantic_from_attribute(ctx, name, txt, &strs, sym, &[], start_line(ann));
        }
    }
}

// =========================================================================
// 10. Swift Extraction
// =========================================================================

pub(super) fn extract_swift(ctx: &mut ExtractCtx<'_>, root: Node<'_>) {
    walk_swift(ctx, root, 0, None, 0);
}

fn walk_swift(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    depth: usize,
    owner_class: Option<&str>,
    fn_depth: usize,
) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    match node.kind() {
        "import_declaration" => {
            let path = ctx
                .text(node)
                .trim_start_matches("import")
                .trim()
                .to_owned();
            let sym = path.rsplit('.').next().unwrap_or(&path).to_owned();
            ctx.push_import(AstImport {
                path,
                symbols: vec![sym],
                alias: None,
                is_wildcard: false,
                is_reexport: false,
                reexport_visibility: None,
                kind: "import".to_owned(),
                line: start_line(node),
                evidence: clip_str(ctx.text(node), 160),
            });
            return;
        }
        "class_declaration" | "protocol_declaration" => {
            let decl_kind = if node.kind() == "protocol_declaration" {
                "protocol"
            } else {
                match node
                    .child_by_field_name("declaration_kind")
                    .map(|k| ctx.text(k).trim())
                {
                    Some("struct") => "struct",
                    Some("enum") => "enum",
                    _ => "class",
                }
            };
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let line = start_line(node);
            let body_node = node.child_by_field_name("body");
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source);
            let (vis, mods) = parse_swift_modifiers(node, ctx.source);
            if !name.is_empty() {
                ctx.push_decl(
                    decl_kind,
                    &name,
                    owner_class.map(str::to_owned),
                    line,
                    end_line(node),
                    Some(vis),
                    mods,
                    sig.clone(),
                    strip_swift_vis(&sig),
                    String::new(),
                );
                for inh in children_by_kind(node, "inheritance_specifier") {
                    let parent = ctx.text(inh).split('<').next().unwrap_or("").trim();
                    ctx.push_inheritance(&name, parent, "conforms", line, ctx.text(inh));
                }
                if let Some(body) = body_node {
                    walk_swift(ctx, body, depth + 1, Some(&name), fn_depth);
                }
            }
            return;
        }
        "function_declaration" | "protocol_function_declaration" | "init_declaration" => {
            if fn_depth > 0 {
                return;
            }
            let name = if node.kind() == "init_declaration" {
                "init".to_owned()
            } else {
                node.child_by_field_name("name")
                    .map(|n| ctx.text(n).trim().to_owned())
                    .unwrap_or_default()
            };
            let body_node = node.child_by_field_name("body");
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source);
            let (vis, mods) = parse_swift_modifiers(node, ctx.source);
            if !name.is_empty() {
                ctx.push_decl(
                    if owner_class.is_some() {
                        "method"
                    } else {
                        "function"
                    },
                    &name,
                    owner_class.map(str::to_owned),
                    start_line(node),
                    end_line(node),
                    Some(vis),
                    mods,
                    sig.clone(),
                    strip_swift_vis(&sig),
                    body_node
                        .map(|b| ast_body_fingerprint(b, ctx.source))
                        .unwrap_or_default(),
                );
            }
            if let Some(body) = body_node {
                walk_swift(ctx, body, depth + 1, owner_class, fn_depth + 1);
            }
            return;
        }
        "property_declaration" | "protocol_property_declaration" => {
            if fn_depth == 0
                && let Some(n) = node.child_by_field_name("name")
            {
                let name = ctx.text(n).trim().to_owned();
                let (vis, mods) = parse_swift_modifiers(node, ctx.source);
                let sig = header_without_comments(
                    node,
                    node.child_by_field_name("computed_value")
                        .map_or(node.end_byte(), |c| c.start_byte()),
                    ctx.source,
                )
                .split('=')
                .next()
                .unwrap_or("")
                .trim()
                .to_owned();
                ctx.push_decl(
                    "property",
                    &name,
                    owner_class.map(str::to_owned),
                    start_line(node),
                    end_line(node),
                    Some(vis),
                    mods,
                    sig.clone(),
                    strip_swift_vis(&sig),
                    String::new(),
                );
            }
            return;
        }
        "call_expression" => {
            let full = ctx.text(node);
            let line = start_line(node);
            let callee_part = full.split('(').next().unwrap_or("").trim();
            let (recv, mem) = match callee_part.rsplit_once('.') {
                Some((r, m)) => (Some(r.trim()), m.trim()),
                None => (None, callee_part),
            };
            let str_args = all_string_arg_texts(node, ctx.source);
            let args_txt = full.split_once('(').map_or("", |(_, r)| r);
            emit_semantic_from_call(
                ctx,
                recv,
                mem,
                args_txt,
                &str_args,
                full,
                owner_class.map(str::to_owned),
                line,
            );
            ctx.push_ref(AstRef {
                target_name: recv.unwrap_or(mem).to_owned(),
                member: recv.is_some().then(|| mem.to_owned()),
                context: "call".to_owned(),
                access: "call".to_owned(),
                member_kind: Some(if recv.is_some() { "method" } else { "function" }.to_owned()),
                owner_type: recv.map(str::to_owned),
                enclosing_symbol: owner_class.map(str::to_owned),
                is_static: false,
                line,
                evidence: clip_str(full, 160),
            });
        }
        _ => {}
    }
    for i in 0..node.child_count() {
        if let Some(ch) = node.child(i) {
            walk_swift(ctx, ch, depth + 1, owner_class, fn_depth);
        }
    }
}

fn parse_swift_modifiers(node: Node<'_>, source: &str) -> (String, Vec<String>) {
    let mut vis = "internal".to_owned();
    let mut mods = Vec::new();
    if let Some(mnode) = child_by_kind(node, "modifiers") {
        for word in node_text(mnode, source).split_whitespace() {
            if matches!(
                word,
                "open" | "public" | "package" | "internal" | "fileprivate" | "private"
            ) {
                vis = if word == "fileprivate" {
                    "private".to_owned()
                } else if word == "open" {
                    "public".to_owned()
                } else {
                    word.to_owned()
                };
            } else {
                push_unique_mod(&mut mods, word);
            }
        }
    }
    (vis, mods)
}

fn strip_swift_vis(sig: &str) -> String {
    let mut parts = Vec::new();
    let mut skip = true;
    for tok in sig.split_whitespace() {
        if skip
            && matches!(
                tok,
                "open" | "public" | "package" | "internal" | "fileprivate" | "private"
            )
        {
            continue;
        }
        skip = false;
        parts.push(tok);
    }
    parts.join(" ")
}

// =========================================================================
// 11. Scala Extraction
// =========================================================================

pub(super) fn extract_scala(ctx: &mut ExtractCtx<'_>, root: Node<'_>) {
    walk_scala(ctx, root, 0, None, 0);
}

fn walk_scala(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    depth: usize,
    owner_class: Option<&str>,
    fn_depth: usize,
) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    match node.kind() {
        "package_clause" => {
            if let Some(name) = node.child_by_field_name("name") {
                ctx.namespace = Some(ctx.text(name).trim().to_owned());
            }
        }
        "import_declaration" | "export_declaration" => {
            let is_reexport = node.kind() == "export_declaration";
            let raw = ctx
                .text(node)
                .trim_start_matches("import")
                .trim_start_matches("export")
                .trim()
                .trim_end_matches(';');
            let is_wc = raw.ends_with("._") || raw.ends_with(".*");
            let clean = raw
                .trim_end_matches("._")
                .trim_end_matches(".*")
                .split('{')
                .next()
                .unwrap_or(raw)
                .trim_end_matches('.');
            let syms: Vec<String> = if let Some((_, inside)) = raw.split_once('{') {
                inside
                    .trim_end_matches('}')
                    .split(',')
                    .map(|s| s.split("=>").next().unwrap_or("").trim().to_owned())
                    .filter(|s| !s.is_empty() && s != "_" && s != "*")
                    .collect()
            } else if !is_wc {
                clean
                    .rsplit('.')
                    .next()
                    .map(|s| vec![s.to_owned()])
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            ctx.push_import(AstImport {
                path: clean.to_owned(),
                symbols: syms,
                alias: None,
                is_wildcard: is_wc,
                is_reexport,
                reexport_visibility: is_reexport.then(|| "public".to_owned()),
                kind: if is_reexport { "reexport" } else { "import" }.to_owned(),
                line: start_line(node),
                evidence: clip_str(ctx.text(node), 160),
            });
            return;
        }
        "class_definition" | "trait_definition" | "object_definition" | "enum_definition" => {
            let decl_kind = match node.kind() {
                "trait_definition" => "trait",
                "enum_definition" => "enum",
                _ => "class",
            };
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let body_node = node.child_by_field_name("body");
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source);
            let (vis, mods) = parse_scala_modifiers(node, ctx.source);
            let line = start_line(node);
            if !name.is_empty() {
                ctx.push_decl(
                    decl_kind,
                    &name,
                    owner_class.map(str::to_owned),
                    line,
                    end_line(node),
                    Some(vis),
                    mods,
                    sig.clone(),
                    strip_leading_vis_word(&sig),
                    String::new(),
                );
                if let Some(ext) = node.child_by_field_name("extend") {
                    let ext_txt = ctx.text(ext).trim_start_matches("extends").trim();
                    for (idx, part) in ext_txt.split(" with ").enumerate() {
                        let base = part.split(['(', '[']).next().unwrap_or("").trim();
                        let ik = if idx == 0 { "extends" } else { "implements" };
                        ctx.push_inheritance(&name, base, ik, line, ctx.text(ext));
                    }
                }
                if let Some(body) = body_node {
                    walk_scala(ctx, body, depth + 1, Some(&name), fn_depth);
                }
            }
            return;
        }
        "function_definition" | "function_declaration" => {
            if fn_depth > 0 {
                return;
            }
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let body_node = node.child_by_field_name("body");
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source);
            let (vis, mods) = parse_scala_modifiers(node, ctx.source);
            if !name.is_empty() {
                ctx.push_decl(
                    if owner_class.is_some() {
                        "method"
                    } else {
                        "function"
                    },
                    &name,
                    owner_class.map(str::to_owned),
                    start_line(node),
                    end_line(node),
                    Some(vis),
                    mods,
                    sig.clone(),
                    strip_leading_vis_word(&sig),
                    body_node
                        .map(|b| ast_body_fingerprint(b, ctx.source))
                        .unwrap_or_default(),
                );
            }
            if let Some(body) = body_node {
                walk_scala(ctx, body, depth + 1, owner_class, fn_depth + 1);
            }
            return;
        }
        "val_definition" | "val_declaration" | "var_definition" | "var_declaration" => {
            if fn_depth == 0 {
                let name = node
                    .child_by_field_name("pattern")
                    .or_else(|| node.child_by_field_name("name"))
                    .map(|n| ctx.text(n).trim().to_owned())
                    .unwrap_or_default();
                let (vis, mods) = parse_scala_modifiers(node, ctx.source);
                let val_node = node.child_by_field_name("value");
                let sig = header_without_comments(
                    node,
                    val_node.map_or(node.end_byte(), |v| v.start_byte()),
                    ctx.source,
                )
                .trim_end_matches('=')
                .trim()
                .to_owned();
                if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    ctx.push_decl(
                        if is_upper_snake(&name) {
                            "const"
                        } else {
                            "property"
                        },
                        &name,
                        owner_class.map(str::to_owned),
                        start_line(node),
                        end_line(node),
                        Some(vis),
                        mods,
                        sig.clone(),
                        strip_leading_vis_word(&sig),
                        val_node
                            .map(|v| ast_body_fingerprint(v, ctx.source))
                            .unwrap_or_default(),
                    );
                }
            }
            return;
        }
        "call_expression" => {
            if let Some(fn_node) = node.child_by_field_name("function") {
                let fn_txt = ctx.text(fn_node).trim();
                let line = start_line(node);
                let evidence = ctx.text(node);
                let args_node = node.child_by_field_name("arguments");
                let args_txt = args_node.map(|a| ctx.text(a)).unwrap_or("");
                let str_args = args_node
                    .map(|a| all_string_arg_texts(a, ctx.source))
                    .unwrap_or_default();
                let (recv, mem) = match fn_txt.rsplit_once('.') {
                    Some((r, m)) => (Some(r.trim()), m.trim()),
                    None => (None, fn_txt),
                };
                emit_semantic_from_call(
                    ctx,
                    recv,
                    mem,
                    args_txt,
                    &str_args,
                    evidence,
                    owner_class.map(str::to_owned),
                    line,
                );
                ctx.push_ref(AstRef {
                    target_name: recv.unwrap_or(mem).to_owned(),
                    member: recv.is_some().then(|| mem.to_owned()),
                    context: "call".to_owned(),
                    access: "call".to_owned(),
                    member_kind: Some(
                        if recv.is_some() { "method" } else { "function" }.to_owned(),
                    ),
                    owner_type: recv.map(str::to_owned),
                    enclosing_symbol: owner_class.map(str::to_owned),
                    is_static: false,
                    line,
                    evidence: clip_str(evidence, 160),
                });
            }
        }
        _ => {}
    }
    for i in 0..node.child_count() {
        if let Some(ch) = node.child(i) {
            walk_scala(ctx, ch, depth + 1, owner_class, fn_depth);
        }
    }
}

fn parse_scala_modifiers(node: Node<'_>, source: &str) -> (String, Vec<String>) {
    let mut vis = "public".to_owned();
    let mut mods = Vec::new();
    if let Some(acc) = child_by_kind(node, "access_modifier") {
        let txt = node_text(acc, source).trim();
        vis = if txt.starts_with("private") {
            "private".to_owned()
        } else if txt.starts_with("protected") {
            "protected".to_owned()
        } else {
            txt.to_owned()
        };
    }
    if let Some(mnode) = child_by_kind(node, "modifiers") {
        for word in node_text(mnode, source).split_whitespace() {
            if word.starts_with("private") {
                vis = "private".to_owned();
            } else if word.starts_with("protected") {
                vis = "protected".to_owned();
            } else {
                push_unique_mod(&mut mods, word);
            }
        }
    }
    (vis, mods)
}
