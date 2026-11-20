use super::*;

/// Synthesizes a normalized cross-service map document from `/api/systems/catalog`
/// so the map renders immediately and gracefully merges with `/api/systems/graph/map`.
pub fn synthesize_map_from_catalog(catalog: &V, graph_status: &V) -> V {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut provider_by_contract: BTreeMap<String, Vec<String>> = BTreeMap::new();

    let services = m::array(&catalog["services"]);
    let status_services = &graph_status["services"];

    for svc in &services {
        let id = m::text(svc, "id");
        if id.is_empty() {
            continue;
        }
        let declared = &svc["declared"];
        let svc_provides = if svc["provides"].is_array() {
            &svc["provides"]
        } else {
            &declared["provides"]
        };
        let svc_consumes = if svc["consumes"].is_array() {
            &svc["consumes"]
        } else {
            &declared["consumes"]
        };
        let svc_capabilities = if svc["capabilities"].is_array() {
            &svc["capabilities"]
        } else {
            &declared["capabilities"]
        };
        for c in m::array(svc_provides) {
            let cname = contract_name(&c);
            if !cname.is_empty() {
                provider_by_contract
                    .entry(cname)
                    .or_default()
                    .push(id.clone());
            }
        }
        let g_status = if status_services.is_object() {
            status_services[&id].clone()
        } else {
            V::Null
        };
        let role = {
            let r = m::text(svc, "role");
            if r.is_empty() {
                m::text(declared, "role")
            } else {
                r
            }
        };
        let owner = {
            let o = m::text(svc, "owner");
            if o.is_empty() {
                m::text(declared, "owner")
            } else {
                o
            }
        };
        let description = {
            let d = m::text(svc, "description");
            if d.is_empty() {
                m::text(declared, "description")
            } else {
                d
            }
        };
        let language = {
            let l = first_non_empty(svc, &["language", "stack"]);
            if l.is_empty() {
                first_non_empty(declared, &["language", "stack"])
            } else {
                l
            }
        };
        nodes.push(json!({
            "id": id,
            "path": id,
            "service": id,
            "kind": "service",
            "name": service_display_name(svc),
            "role": role,
            "owner": owner,
            "description": description,
            "folder": m::text(svc, "root").chars().next().map(|_| m::text(svc, "root")).unwrap_or_else(|| m::text(svc, "path")),
            "language": language,
            "capabilities": svc_capabilities,
            "provides": svc_provides,
            "consumes": svc_consumes,
            "memory": svc["memory"],
            "brain": svc["brain"],
            "git": svc["git"],
            "graph_status": g_status,
            "warnings": svc["warnings"],
        }));
    }

    let shared_sources = m::array(&catalog["shared_sources"]);
    let has_system_graph = status_services.is_object() && status_services["__system__"].is_object();
    let sys_str_name = catalog["system"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    if !shared_sources.is_empty()
        || has_system_graph
        || catalog["system"].is_object()
        || sys_str_name.is_some()
    {
        let sys_name = {
            let n = sys_str_name.unwrap_or_else(|| m::text(&catalog["system"], "name"));
            if n.is_empty() {
                "System shared context".to_owned()
            } else {
                format!("{n} (shared)")
            }
        };
        let sys_folder = if catalog["system"].is_object() {
            m::text(&catalog["system"], "root")
        } else {
            String::new()
        };
        nodes.push(json!({
            "id": "__system__",
            "path": "__system__",
            "service": "__system__",
            "kind": "system",
            "name": sys_name,
            "role": "shared-authority",
            "folder": sys_folder,
            "shared_sources": shared_sources,
            "graph_status": if status_services.is_object() { status_services["__system__"].clone() } else { V::Null },
        }));
    }

    let rels = m::array(&catalog["relationships"]);
    let mut pair_edges: BTreeMap<(String, String), V> = BTreeMap::new();

    if !rels.is_empty() {
        for rel in &rels {
            let from = first_non_empty(rel, &["consumer", "from", "source"]);
            let to = first_non_empty(rel, &["provider", "to", "target"]);
            if from.is_empty() || to.is_empty() || from == to {
                continue;
            }
            let cname = first_non_empty(rel, &["contract", "name", "id"]);
            let kind = normalize_contract_arrow_kind(&first_non_empty(rel, &["kind", "type"]));
            let broken = m::yes(&rel["unmatched"])
                || m::yes(&rel["broken_risk"])
                || m::yes(&rel["version_mismatch"]);
            let entry = pair_edges
                .entry((from.clone(), to.clone()))
                .or_insert_with(|| {
                    json!({
                        "from": from,
                        "to": to,
                        "kind": kind,
                        "kinds": [],
                        "contracts": [],
                        "provenance": "declared",
                        "broken_risk": false,
                        "against": false,
                    })
                });
            if broken {
                entry["broken_risk"] = json!(true);
            }
            push_unique_str(&mut entry["kinds"], kind);
            if !cname.is_empty() {
                push_contract_obj(
                    &mut entry["contracts"],
                    json!({
                        "name": cname,
                        "kind": kind,
                        "version": m::text(rel, "version"),
                        "broken_risk": broken,
                    }),
                );
            }
        }
    } else {
        for svc in &services {
            let consumer = m::text(svc, "id");
            if consumer.is_empty() {
                continue;
            }
            let svc_consumes = if svc["consumes"].is_array() {
                &svc["consumes"]
            } else {
                &svc["declared"]["consumes"]
            };
            for c in m::array(svc_consumes) {
                let raw = contract_name(&c);
                if raw.is_empty() {
                    continue;
                }
                let explicit_svc = first_non_empty(&c, &["service", "provider"]);
                let (explicit_prov, cname) = if !explicit_svc.is_empty() {
                    (Some(explicit_svc), raw.clone())
                } else if let Some((p, n)) = raw.split_once(':') {
                    (Some(p.trim().to_owned()), n.trim().to_owned())
                } else {
                    (None, raw.clone())
                };
                let kind = normalize_contract_arrow_kind(&m::text(&c, "kind"));
                let providers = if let Some(p) = explicit_prov {
                    vec![p]
                } else {
                    provider_by_contract
                        .get(&cname)
                        .cloned()
                        .unwrap_or_default()
                };
                if providers.is_empty() {
                    let target = if nodes.iter().any(|n| n["id"] == "__system__") {
                        "__system__".to_owned()
                    } else {
                        continue;
                    };
                    let entry = pair_edges
                        .entry((consumer.clone(), target.clone()))
                        .or_insert_with(|| {
                            json!({
                                "from": consumer,
                                "to": target,
                                "kind": kind,
                                "kinds": [kind],
                                "contracts": [],
                                "provenance": "declared",
                                "broken_risk": true,
                                "against": false,
                            })
                        });
                    entry["broken_risk"] = json!(true);
                    push_contract_obj(
                        &mut entry["contracts"],
                        json!({
                            "name": cname,
                            "kind": kind,
                            "unmatched": true,
                            "broken_risk": true,
                        }),
                    );
                } else {
                    for prov in providers {
                        if prov == consumer {
                            continue;
                        }
                        let entry = pair_edges
                            .entry((consumer.clone(), prov.clone()))
                            .or_insert_with(|| {
                                json!({
                                    "from": consumer,
                                    "to": prov,
                                    "kind": kind,
                                    "kinds": [],
                                    "contracts": [],
                                    "provenance": "declared",
                                    "broken_risk": false,
                                    "against": false,
                                })
                            });
                        push_unique_str(&mut entry["kinds"], kind);
                        push_contract_obj(
                            &mut entry["contracts"],
                            json!({
                                "name": cname,
                                "kind": kind,
                                "version": m::text(&c, "version"),
                            }),
                        );
                    }
                }
            }
        }
    }

    if !shared_sources.is_empty() && nodes.iter().any(|n| n["id"] == "__system__") {
        for svc in &services {
            let sid = m::text(svc, "id");
            if sid.is_empty() {
                continue;
            }
            if svc["shared_sources"]
                .as_array()
                .is_some_and(|arr| !arr.is_empty())
            {
                let entry = pair_edges
                    .entry((sid.clone(), "__system__".to_owned()))
                    .or_insert_with(|| {
                        json!({
                            "from": sid,
                            "to": "__system__",
                            "kind": "shared",
                            "kinds": ["shared"],
                            "contracts": [],
                            "provenance": "declared",
                            "broken_risk": false,
                            "against": false,
                        })
                    });
                push_unique_str(&mut entry["kinds"], "shared");
            }
        }
    }

    edges.extend(pair_edges.into_values());
    json!({
        "nodes": nodes,
        "edges": edges,
        "synthesized": true,
    })
}

/// Normalizes a server `/api/systems/graph/map` document and enriches it with
/// catalog metadata + live service graph statuses.
pub fn normalize_map_doc(raw: &V, catalog: &V, graph_status: &V, state: &V) -> V {
    let base = synthesize_map_from_catalog(catalog, graph_status);
    if !raw.is_object() || (raw["nodes"].is_null() && raw["services"].is_null()) {
        return expand_unfolded_nodes(&base, state);
    }

    let mut cat_by_id = BTreeMap::new();
    for node in m::array(&base["nodes"]) {
        let id = node_id(&node);
        if !id.is_empty() {
            cat_by_id.insert(id, node);
        }
    }

    let raw_nodes = if raw["nodes"].is_array() {
        m::array(&raw["nodes"])
    } else {
        m::array(&raw["services"])
    };

    let status_services = &graph_status["services"];
    let mut nodes = Vec::new();
    let mut seen = BTreeSet::new();

    for mut n in raw_nodes {
        let id = node_id(&n);
        if id.is_empty() || !seen.insert(id.clone()) {
            continue;
        }
        if n["name"].is_null() || n["name"] == "" {
            let lbl = m::text(&n, "label");
            if !lbl.is_empty() {
                n["name"] = json!(lbl);
            }
        }
        if n["language"].is_null() || n["language"] == "" {
            let stack = m::text(&n, "stack");
            if !stack.is_empty() {
                n["language"] = json!(stack);
            }
        }
        if n["folder"].is_null() || n["folder"] == "" {
            let root = m::text(&n, "root");
            if !root.is_empty() {
                n["folder"] = json!(root);
            }
        }
        if n["parent_service"].is_null() || n["parent_service"] == "" {
            let parent = m::text(&n, "parent");
            if !parent.is_empty() {
                n["parent_service"] = json!(parent);
            }
        }
        if n["graph_status"].is_null() && n["code_graph"].is_object() {
            n["graph_status"] = n["code_graph"].clone();
        }
        if let Some(cat) = cat_by_id.get(&id) {
            for field in [
                "name",
                "role",
                "owner",
                "description",
                "folder",
                "language",
                "capabilities",
                "provides",
                "consumes",
                "memory",
                "brain",
                "git",
                "shared_sources",
                "warnings",
            ] {
                if n[field].is_null() || n[field] == "" {
                    n[field] = cat[field].clone();
                }
            }
        }
        if status_services.is_object() && status_services[&id].is_object() {
            n["graph_status"] = status_services[&id].clone();
        }
        if n["id"].is_null() {
            n["id"] = json!(id.clone());
        }
        if n["path"].is_null() {
            n["path"] = json!(id.clone());
        }
        if n["kind"].is_null() {
            n["kind"] = json!(if id == "__system__" {
                "system"
            } else {
                "service"
            });
        }
        nodes.push(n);
    }

    for (id, cat_node) in cat_by_id {
        if !seen.contains(&id) && raw["filtered"].as_bool() != Some(true) {
            nodes.push(cat_node);
        }
    }

    let raw_edges = if raw["edges"].is_array() {
        m::array(&raw["edges"])
    } else if raw["links"].is_array() {
        m::array(&raw["links"])
    } else {
        m::array(&raw["relationships"])
    };

    let mut edges = Vec::new();
    for mut e in raw_edges {
        let from = first_non_empty(&e, &["from", "consumer", "source"]);
        let to = first_non_empty(&e, &["to", "provider", "target"]);
        if from.is_empty() || to.is_empty() {
            continue;
        }
        e["from"] = json!(from);
        e["to"] = json!(to);
        if e["level"].as_str() == Some("detail") {
            e["child_link"] = json!(true);
        }
        let pkind = primary_edge_kind(&e);
        e["kind"] = json!(pkind);
        let prov = first_non_empty(&e, &["provenance", "verification", "status"]);
        if !prov.is_empty() {
            e["provenance"] = json!(prov);
        } else if m::yes(&e["undeclared"]) || m::yes(&e["undeclared_in_code"]) {
            e["provenance"] = json!("undeclared_in_code");
        } else if m::yes(&e["verified"]) || m::yes(&e["confirmed"]) {
            e["provenance"] = json!("verified");
        } else {
            e["provenance"] = json!("declared");
        }
        let broken = m::yes(&e["broken_risk"])
            || m::yes(&e["unmatched"])
            || m::yes(&e["version_mismatch"])
            || m::yes(&e["drift"]);
        e["broken_risk"] = json!(broken);
        let against = m::yes(&e["against"]) || m::yes(&e["violation"]);
        e["against"] = json!(against);
        edges.push(e);
    }

    if edges.is_empty() && !raw["edges"].is_array() {
        edges = m::array(&base["edges"]);
    }

    for risk in m::array(&raw["contract_risks"]) {
        let consumer = first_non_empty(&risk, &["consumer", "from", "service"]);
        let provider = first_non_empty(&risk, &["provider", "to"]);
        let cname = first_non_empty(&risk, &["contract", "name", "id"]);
        let rkind = normalize_contract_arrow_kind(&m::text(&risk, "kind"));
        if consumer.is_empty() {
            continue;
        }
        if !provider.is_empty() {
            let mut matched = false;
            for e in &mut edges {
                if e["from"].as_str() == Some(&consumer) && e["to"].as_str() == Some(&provider) {
                    e["broken_risk"] = json!(true);
                    matched = true;
                }
            }
            if !matched {
                edges.push(json!({
                    "from": consumer,
                    "to": provider,
                    "kind": rkind,
                    "kinds": [rkind],
                    "contracts": [{ "name": cname, "kind": rkind }],
                    "provenance": "declared",
                    "broken_risk": true,
                    "against": false,
                }));
            }
        } else if !cname.is_empty() {
            let un_id = format!("__unmatched__:{cname}");
            if !nodes.iter().any(|n| node_id(n) == un_id) {
                nodes.push(json!({
                    "id": un_id,
                    "path": un_id,
                    "kind": "contract",
                    "name": format!("? {cname}"),
                    "unmatched": true,
                }));
            }
            if !edges
                .iter()
                .any(|e| e["from"].as_str() == Some(&consumer) && e["to"].as_str() == Some(&un_id))
            {
                edges.push(json!({
                    "from": consumer,
                    "to": un_id,
                    "kind": rkind,
                    "kinds": [rkind],
                    "contracts": [{ "name": cname, "kind": rkind }],
                    "provenance": "declared",
                    "broken_risk": true,
                    "against": false,
                }));
            }
        }
    }

    let mut out = raw.clone();
    out["nodes"] = json!(nodes);
    out["edges"] = json!(edges);
    expand_unfolded_nodes(&out, state)
}

pub(super) fn is_child_node_kind(kind: &str) -> bool {
    matches!(kind, "contract" | "capability" | "source" | "more")
}

pub(super) const UNFOLD_CAP_LIMIT: usize = 6;
pub(super) const UNFOLD_CONTRACT_LIMIT: usize = 8;
pub(super) const UNFOLD_SOURCE_LIMIT: usize = 6;

pub(super) fn expand_unfolded_nodes(doc: &V, state: &V) -> V {
    let whole = m::yes(&state["whole"]);
    let spread_set: BTreeSet<String> = m::array(&state["spread"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    if !whole && spread_set.is_empty() {
        return doc.clone();
    }

    let base_nodes = m::array(&doc["nodes"]);
    let mut existing_ids: BTreeSet<String> = BTreeSet::new();
    let mut existing_by_svc_kind: BTreeMap<(String, &'static str), BTreeSet<String>> =
        BTreeMap::new();
    let mut contract_node_by_key: BTreeMap<(String, String), String> = BTreeMap::new();

    for n in &base_nodes {
        let nid = node_id(n);
        if nid.is_empty() {
            continue;
        }
        existing_ids.insert(nid.clone());
        let kind_str = n["kind"].as_str().unwrap_or("");
        if !nid.starts_with("__unmatched__:")
            && !m::yes(&n["unmatched"])
            && is_child_node_kind(kind_str)
        {
            let parent = first_non_empty(n, &["parent_service", "parent", "service"]);
            if !parent.is_empty() {
                let kind_key = match kind_str {
                    "capability" => "capability",
                    "contract" => "contract",
                    "source" => "source",
                    _ => "more",
                };
                let label = first_non_empty(n, &["name", "label", "path", "id"]);
                existing_by_svc_kind
                    .entry((parent.clone(), kind_key))
                    .or_default()
                    .insert(label.clone());
                if kind_key == "contract" && !label.is_empty() {
                    contract_node_by_key.insert((parent, label), nid);
                }
            }
        }
    }

    let mut nodes = base_nodes;
    let mut edges = m::array(&doc["edges"]);
    let mut new_nodes: Vec<V> = Vec::new();
    let mut svc_overflow_updates: Vec<(usize, usize)> = Vec::new();

    for (svc_idx, svc) in nodes.iter().enumerate() {
        let skind = svc["kind"].as_str().unwrap_or("service");
        if is_child_node_kind(skind) {
            continue;
        }
        let sid = node_id(svc);
        if sid.is_empty()
            || sid.starts_with("__unmatched__:")
            || m::yes(&svc["unmatched"])
            || (!whole && !spread_set.contains(&sid))
        {
            continue;
        }

        // Capabilities
        let mut unique_caps: Vec<String> = Vec::new();
        for cap in m::array(&svc["capabilities"]) {
            let cname = capability_name(&cap);
            if !cname.trim().is_empty() && !unique_caps.contains(&cname) {
                unique_caps.push(cname);
            }
        }
        let existing_caps = existing_by_svc_kind
            .get(&(sid.clone(), "capability"))
            .cloned()
            .unwrap_or_default();
        let mut cap_slots_used = existing_caps.len();
        let cap_total = unique_caps.len().max(cap_slots_used);
        if cap_slots_used == 0 {
            for cname in unique_caps.iter().take(UNFOLD_CAP_LIMIT) {
                let cid = format!("{sid}::cap::{cname}");
                if existing_ids.insert(cid.clone()) {
                    new_nodes.push(json!({
                        "id": cid,
                        "path": cid,
                        "service": sid,
                        "parent_service": sid,
                        "kind": "capability",
                        "name": cname,
                    }));
                    cap_slots_used += 1;
                }
            }
        }
        let caps_more = cap_total.saturating_sub(cap_slots_used);

        // Contracts (provides)
        let mut unique_contracts: Vec<(String, V)> = Vec::new();
        for c in m::array(&svc["provides"]) {
            let cname = contract_name(&c);
            if !cname.is_empty() && !unique_contracts.iter().any(|(n, _)| n == &cname) {
                unique_contracts.push((cname, c));
            }
        }
        let existing_contracts = existing_by_svc_kind
            .get(&(sid.clone(), "contract"))
            .cloned()
            .unwrap_or_default();
        let mut contract_slots_used = existing_contracts.len();
        let contracts_total = unique_contracts.len().max(contract_slots_used);
        if contract_slots_used == 0 {
            for (cname, c) in unique_contracts.iter().take(UNFOLD_CONTRACT_LIMIT) {
                let ckind = normalize_contract_arrow_kind(&m::text(c, "kind"));
                let cid = format!("{sid}::contract::{cname}");
                contract_node_by_key.insert((sid.clone(), cname.clone()), cid.clone());
                if existing_ids.insert(cid.clone()) {
                    new_nodes.push(json!({
                        "id": cid,
                        "path": cid,
                        "service": sid,
                        "parent_service": sid,
                        "kind": "contract",
                        "contract_kind": ckind,
                        "name": cname,
                        "version": m::text(c, "version"),
                        "source": first_non_empty(c, &["source", "schema", "path"]),
                    }));
                    contract_slots_used += 1;
                }
            }
        }
        let contracts_more = contracts_total.saturating_sub(contract_slots_used);

        // Shared sources
        let mut unique_sources: Vec<String> = Vec::new();
        for src in m::array(&svc["shared_sources"]) {
            let spath = src
                .as_str()
                .map(str::trim)
                .map(str::to_owned)
                .unwrap_or_else(|| m::text(&src, "path"));
            if !spath.is_empty() && !unique_sources.contains(&spath) {
                unique_sources.push(spath);
            }
        }
        let existing_sources = existing_by_svc_kind
            .get(&(sid.clone(), "source"))
            .cloned()
            .unwrap_or_default();
        let mut source_slots_used = existing_sources.len();
        let sources_total = unique_sources.len().max(source_slots_used);
        if source_slots_used == 0 {
            for spath in unique_sources.iter().take(UNFOLD_SOURCE_LIMIT) {
                let cid = format!("{sid}::source::{spath}");
                if existing_ids.insert(cid.clone()) {
                    new_nodes.push(json!({
                        "id": cid,
                        "path": spath,
                        "service": sid,
                        "parent_service": sid,
                        "kind": "source",
                        "name": spath.rsplit('/').next().unwrap_or(spath),
                    }));
                    source_slots_used += 1;
                }
            }
        }
        let sources_more = sources_total.saturating_sub(source_slots_used);

        let backend_more = svc["unfolded_more"].as_u64().unwrap_or(0) as usize;
        let overflow = caps_more + contracts_more + sources_more + backend_more;
        let more_id = format!("{sid}::more");
        if overflow > 0
            && !existing_by_svc_kind.contains_key(&(sid.clone(), "more"))
            && existing_ids.insert(more_id.clone())
        {
            let mut parts = Vec::new();
            if caps_more > 0 {
                parts.push(format!("{caps_more} capabilities"));
            }
            if contracts_more > 0 {
                parts.push(format!("{contracts_more} contracts"));
            }
            if sources_more > 0 {
                parts.push(format!("{sources_more} sources"));
            }
            if backend_more > 0 {
                parts.push(format!("{backend_more} items"));
            }
            let detail = format!(
                "+{} hidden ({}) — click to view all",
                overflow,
                parts.join(", ")
            );
            new_nodes.push(json!({
                "id": more_id,
                "path": sid,
                "service": sid,
                "parent_service": sid,
                "kind": "more",
                "name": format!("+{overflow} more"),
                "detail": detail,
                "more_count": overflow,
            }));
            svc_overflow_updates.push((svc_idx, overflow));
        }
    }

    for (svc_idx, overflow) in svc_overflow_updates {
        if let Some(svc) = nodes.get_mut(svc_idx) {
            svc["unfolded_truncated"] = json!(true);
            svc["unfolded_more"] = json!(overflow);
        }
    }
    nodes.extend(new_nodes);

    let mut existing_edge_pairs: BTreeSet<(String, String)> = edges
        .iter()
        .map(|e| (m::text(e, "from"), m::text(e, "to")))
        .collect();
    for edge in m::array(&doc["edges"]) {
        let from = m::text(&edge, "from");
        let to = m::text(&edge, "to");
        for c in m::array(&edge["contracts"]) {
            let cname = contract_name(&c);
            if let Some(cid) = contract_node_by_key.get(&(to.clone(), cname.clone())) {
                if existing_edge_pairs.insert((from.clone(), cid.clone())) {
                    edges.push(json!({
                        "from": from,
                        "to": cid,
                        "kind": normalize_contract_arrow_kind(&m::text(&c, "kind")),
                        "contracts": [c],
                        "provenance": edge["provenance"],
                        "broken_risk": edge["broken_risk"],
                        "against": edge["against"],
                        "child_link": true,
                    }));
                }
            }
        }
    }

    let mut out = doc.clone();
    out["nodes"] = json!(nodes);
    out["edges"] = json!(edges);
    out
}

/// Computes cross-service blast radius from `seed_services` + `changed_contracts` up to `depth` hops.
/// In our graph convention, `edge.from` consumes `edge.to` (`consumer -> provider`).
/// Therefore when a provider `P` changes, downstream impacted consumers are nodes `C` with an edge `C -> P`!
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlastRadius {
    pub seeds: BTreeSet<String>,
    pub direct_consumers: BTreeSet<String>,
    pub downstream_consumers: BTreeSet<String>,
    pub matched_contracts: BTreeSet<String>,
    pub wave_by_service: BTreeMap<String, usize>,
}

pub fn compute_blast_radius(
    doc: &V,
    seed_services: &[String],
    changed_contracts: &[String],
    depth: usize,
) -> BlastRadius {
    let mut result = BlastRadius::default();
    let nodes = m::array(&doc["nodes"]);
    let edges = m::array(&doc["edges"]);

    for s in seed_services {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            result.seeds.insert(trimmed.to_owned());
        }
    }

    for raw_c in changed_contracts {
        let token = raw_c.trim();
        if token.is_empty() {
            continue;
        }
        let (prov_opt, cname) = if let Some((p, c)) = token.split_once(':') {
            (Some(p.trim()), c.trim())
        } else {
            (None, token)
        };
        if !cname.is_empty() {
            result.matched_contracts.insert(cname.to_owned());
        }
        if let Some(p) = prov_opt.filter(|p| !p.is_empty()) {
            result.seeds.insert(p.to_owned());
        } else {
            for node in &nodes {
                let sid = node_id(node);
                if m::array(&node["provides"])
                    .iter()
                    .any(|p| contract_name(p).eq_ignore_ascii_case(cname))
                {
                    result.seeds.insert(sid);
                }
            }
        }
    }

    // Build provider -> consumers adjacency (reverse of consumer -> provider edges).
    let mut consumers_of: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for edge in &edges {
        if m::yes(&edge["child_link"]) {
            continue;
        }
        let consumer = m::text(edge, "from");
        let provider = m::text(edge, "to");
        if !consumer.is_empty() && !provider.is_empty() && consumer != "__system__" {
            consumers_of.entry(provider).or_default().push(consumer);
        }
    }

    let mut queue = VecDeque::new();
    for seed in &result.seeds {
        result.wave_by_service.insert(seed.clone(), 0);
        queue.push_back((seed.clone(), 0usize));
    }

    while let Some((curr, dist)) = queue.pop_front() {
        if dist >= depth {
            continue;
        }
        if let Some(list) = consumers_of.get(&curr) {
            for next in list {
                let next_dist = dist + 1;
                if !result.wave_by_service.contains_key(next) {
                    result.wave_by_service.insert(next.clone(), next_dist);
                    if next_dist == 1 {
                        result.direct_consumers.insert(next.clone());
                    } else {
                        result.downstream_consumers.insert(next.clone());
                    }
                    queue.push_back((next.clone(), next_dist));
                }
            }
        }
    }

    result
}

/// The waves of the change a picture shows, as its overlay asks for them.
pub(super) fn blast_of(doc: &V, state: &V) -> BlastRadius {
    let overlay = &state["overlay"];
    let texts = |key: &str| {
        m::array(&overlay[key])
            .into_iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect::<Vec<_>>()
    };
    compute_blast_radius(
        doc,
        &texts("seed_services"),
        &texts("changed_contracts"),
        overlay["depth"].as_u64().unwrap_or(32) as usize,
    )
}
/// How many waves a change takes to reach every service it reaches.
/// Detects cross-service dependency cycles using Tarjan's strongly connected components algorithm.
pub fn find_cross_service_cycles(doc: &V) -> Vec<Vec<String>> {
    let mut service_ids = Vec::new();
    let mut id_to_idx = BTreeMap::new();
    for node in m::array(&doc["nodes"]) {
        if node["kind"].as_str().is_some_and(is_child_node_kind) {
            continue;
        }
        let id = node_id(&node);
        if !id.is_empty() && id != "__system__" && !id_to_idx.contains_key(&id) {
            id_to_idx.insert(id.clone(), service_ids.len());
            service_ids.push(id);
        }
    }
    let n = service_ids.len();
    let mut adj = vec![Vec::new(); n];
    for edge in m::array(&doc["edges"]) {
        if m::yes(&edge["child_link"]) {
            continue;
        }
        if let (Some(&u), Some(&v)) = (
            id_to_idx.get(&m::text(&edge, "from")),
            id_to_idx.get(&m::text(&edge, "to")),
        ) {
            if u != v && !adj[u].contains(&v) {
                adj[u].push(v);
            }
        }
    }

    struct Tarjan<'a> {
        adj: &'a [Vec<usize>],
        index: usize,
        indices: Vec<Option<usize>>,
        lowlink: Vec<usize>,
        on_stack: Vec<bool>,
        stack: Vec<usize>,
        sccs: Vec<Vec<usize>>,
    }

    fn strongconnect(v: usize, t: &mut Tarjan<'_>) {
        t.indices[v] = Some(t.index);
        t.lowlink[v] = t.index;
        t.index += 1;
        t.stack.push(v);
        t.on_stack[v] = true;

        for &w in &t.adj[v] {
            if t.indices[w].is_none() {
                strongconnect(w, t);
                t.lowlink[v] = t.lowlink[v].min(t.lowlink[w]);
            } else if t.on_stack[w] {
                t.lowlink[v] = t.lowlink[v].min(t.indices[w].unwrap_or(0));
            }
        }

        if Some(t.lowlink[v]) == t.indices[v] {
            let mut comp = Vec::new();
            while let Some(w) = t.stack.pop() {
                t.on_stack[w] = false;
                comp.push(w);
                if w == v {
                    break;
                }
            }
            if comp.len() > 1 {
                comp.reverse();
                t.sccs.push(comp);
            }
        }
    }

    let mut tarjan = Tarjan {
        adj: &adj,
        index: 0,
        indices: vec![None; n],
        lowlink: vec![0; n],
        on_stack: vec![false; n],
        stack: Vec::new(),
        sccs: Vec::new(),
    };
    for v in 0..n {
        if tarjan.indices[v].is_none() {
            strongconnect(v, &mut tarjan);
        }
    }
    tarjan
        .sccs
        .into_iter()
        .map(|comp| comp.into_iter().map(|i| service_ids[i].clone()).collect())
        .collect()
}

/// Computes shortest directed cross-service paths from `from_id` to `to_id`, skipping `skip_ids`.
pub fn shortest_service_paths(
    doc: &V,
    from_id: &str,
    to_id: &str,
    skip_ids: &BTreeSet<String>,
) -> Vec<Vec<String>> {
    if from_id.is_empty() || to_id.is_empty() || from_id == to_id {
        return Vec::new();
    }
    let mut adj: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for edge in m::array(&doc["edges"]) {
        if m::yes(&edge["child_link"]) {
            continue;
        }
        let u = m::text(&edge, "from");
        let v = m::text(&edge, "to");
        if u.is_empty() || v.is_empty() || u == v {
            continue;
        }
        if (u != from_id && u != to_id && skip_ids.contains(&u))
            || (v != from_id && v != to_id && skip_ids.contains(&v))
        {
            continue;
        }
        let entry = adj.entry(u).or_default();
        if !entry.contains(&v) {
            entry.push(v);
        }
    }

    let mut queue = VecDeque::new();
    queue.push_back(vec![from_id.to_owned()]);
    let mut best_len: Option<usize> = None;
    let mut found = Vec::new();
    let mut visited_depth: BTreeMap<String, usize> = BTreeMap::new();
    visited_depth.insert(from_id.to_owned(), 0);

    while let Some(path) = queue.pop_front() {
        if let Some(best) = best_len {
            if path.len() > best {
                break;
            }
        }
        let last = path.last().expect("non-empty path");
        if last == to_id {
            best_len = Some(path.len());
            found.push(path);
            if found.len() >= 4 {
                break;
            }
            continue;
        }
        if path.len() >= 10 {
            continue;
        }
        if let Some(neighbors) = adj.get(last) {
            for next in neighbors {
                if path.contains(next) {
                    continue;
                }
                let next_depth = path.len();
                if visited_depth
                    .get(next)
                    .is_none_or(|&prev| next_depth <= prev)
                {
                    visited_depth.insert(next.clone(), next_depth);
                    let mut next_path = path.clone();
                    next_path.push(next.clone());
                    queue.push_back(next_path);
                }
            }
        }
    }
    found
}

/// Computes `both_use` (shared upstream providers) and `uses_both` (shared downstream consumers) for two services.
pub fn shared_neighbors(doc: &V, a: &str, b: &str) -> (Vec<String>, Vec<String>) {
    if a.is_empty() || b.is_empty() || a == b {
        return (Vec::new(), Vec::new());
    }
    let mut uses_a = BTreeSet::new();
    let mut uses_b = BTreeSet::new();
    let mut used_by_a = BTreeSet::new();
    let mut used_by_b = BTreeSet::new();

    for edge in m::array(&doc["edges"]) {
        if m::yes(&edge["child_link"]) {
            continue;
        }
        let from = m::text(&edge, "from");
        let to = m::text(&edge, "to");
        if from == a {
            uses_a.insert(to.clone());
        }
        if from == b {
            uses_b.insert(to.clone());
        }
        if to == a {
            used_by_a.insert(from.clone());
        }
        if to == b {
            used_by_b.insert(from.clone());
        }
    }
    let both_use = uses_a.intersection(&uses_b).cloned().collect();
    let uses_both = used_by_a.intersection(&used_by_b).cloned().collect();
    (both_use, uses_both)
}

/// Assigns architectural tiers (`0 = Edge / Gateway`, `1 = Application / Workflow`, `2 = Core Domain`, `3 = Shared Authority`)
/// and detects upward cross-tier violations.
pub fn assign_service_layers(doc: &V) -> (BTreeMap<String, usize>, Vec<(String, String)>) {
    let nodes = m::array(&doc["nodes"]);
    let edges = m::array(&doc["edges"]);
    let mut incoming: BTreeMap<String, usize> = BTreeMap::new();
    let mut outgoing: BTreeMap<String, usize> = BTreeMap::new();
    let mut ids = Vec::new();

    for n in &nodes {
        if n["kind"].as_str().is_some_and(is_child_node_kind) {
            continue;
        }
        let id = node_id(n);
        if !id.is_empty() {
            incoming.entry(id.clone()).or_insert(0);
            outgoing.entry(id.clone()).or_insert(0);
            ids.push((id, m::text(n, "role").to_ascii_lowercase()));
        }
    }

    for e in &edges {
        if m::yes(&e["child_link"]) {
            continue;
        }
        let from = m::text(e, "from");
        let to = m::text(e, "to");
        if from != to && incoming.contains_key(&from) && incoming.contains_key(&to) {
            *outgoing.entry(from).or_default() += 1;
            *incoming.entry(to).or_default() += 1;
        }
    }

    let mut tiers = BTreeMap::new();
    for (id, role) in &ids {
        let tier = if id == "__system__" || role.contains("shared") {
            3
        } else if role.contains("gateway")
            || role.contains("edge")
            || role.contains("bff")
            || id.contains("gateway")
            || (incoming[id] == 0 && outgoing[id] > 0)
        {
            0
        } else if outgoing[id] == 0
            || role.contains("identity")
            || role.contains("auth")
            || role.contains("platform")
            || role.contains("core")
        {
            2
        } else {
            1
        };
        tiers.insert(id.clone(), tier);
    }

    let mut violations = Vec::new();
    for e in &edges {
        if m::yes(&e["child_link"]) {
            continue;
        }
        let from = m::text(e, "from");
        let to = m::text(e, "to");
        if let (Some(&tf), Some(&tt)) = (tiers.get(&from), tiers.get(&to)) {
            // Normal flow goes left-to-right (lower tier index -> higher/equal tier index).
            if tf > tt && to != "__system__" {
                violations.push((from, to));
            }
        }
    }
    (tiers, violations)
}

/// Builds a concise, human- and agent-readable system topology / blast-radius brief
/// suitable for attaching straight into `#system-task` in `Changes`.
pub fn build_context_brief(doc: &V, state: &V) -> String {
    let overlay = &state["overlay"];
    let seeds: Vec<String> = m::array(&overlay["seed_services"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let contracts: Vec<String> = m::array(&overlay["changed_contracts"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let depth = overlay["depth"].as_u64().unwrap_or(32) as usize;
    let blast = compute_blast_radius(doc, &seeds, &contracts, depth);

    let mut lines = Vec::new();
    lines.push("[System Architecture & Blast-Radius Context]".to_owned());

    let focus = m::text(state, "focus");
    if !focus.is_empty() {
        if let Some(node) = m::array(&doc["nodes"])
            .into_iter()
            .find(|n| node_id(n) == focus)
        {
            let provides = m::array(&node["provides"])
                .iter()
                .map(contract_name)
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>();
            let consumes = m::array(&node["consumes"])
                .iter()
                .map(contract_name)
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>();
            lines.push(format!(
                "- Focused service: {} ({})",
                m::text(&node, "name"),
                focus
            ));
            if !provides.is_empty() {
                lines.push(format!("  Provides contracts: {}", provides.join(", ")));
            }
            if !consumes.is_empty() {
                lines.push(format!("  Consumes contracts: {}", consumes.join(", ")));
            }
        }
    }

    if !blast.seeds.is_empty() {
        let seed_list = blast.seeds.iter().cloned().collect::<Vec<_>>().join(", ");
        lines.push(format!(
            "- Starting seed services: {seed_list} (depth {depth})"
        ));
        if !blast.direct_consumers.is_empty() {
            lines.push(format!(
                "- Direct contract consumers: {}",
                blast
                    .direct_consumers
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !blast.downstream_consumers.is_empty() {
            lines.push(format!(
                "- Transitive downstream wave: {}",
                blast
                    .downstream_consumers
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }

    if let Some(sel) = state["selected_edge"].as_object() {
        let from = sel.get("from").and_then(V::as_str).unwrap_or("");
        let to = sel.get("to").and_then(V::as_str).unwrap_or("");
        if !from.is_empty() && !to.is_empty() {
            lines.push(format!("- Selected contract link: {from} -> {to}"));
        }
    }

    let broken_edges = m::array(&doc["edges"])
        .into_iter()
        .filter(|e| m::yes(&e["broken_risk"]) || e["provenance"] == "undeclared_in_code")
        .take(4)
        .map(|e| format!("{} -> {}", m::text(&e, "from"), m::text(&e, "to")))
        .collect::<Vec<_>>();
    if !broken_edges.is_empty() {
        lines.push(format!(
            "- Contract risk / drift links to verify: {}",
            broken_edges.join(", ")
        ));
    }

    if lines.len() == 1 {
        let svcs = m::array(&doc["nodes"])
            .into_iter()
            .filter(|n| n["kind"] == "service")
            .map(|n| node_id(&n))
            .collect::<Vec<_>>();
        lines.push(format!(
            "- Member services ({}): {}",
            svcs.len(),
            svcs.join(", ")
        ));
    }

    lines.join("\n")
}
pub fn system_structure_digest(doc: &V, state: &V, pins: &BTreeMap<String, (f64, f64)>) -> u64 {
    let mut hasher = DefaultHasher::new();
    m::yes(&state["layers"]).hash(&mut hasher);
    m::yes(&state["whole"]).hash(&mut hasher);
    m::yes(&state["certain"]).hash(&mut hasher);
    m::text(state, "filter").hash(&mut hasher);
    m::yes(&state["isolate_active"]).hash(&mut hasher);
    state["isolate_depth"]
        .as_u64()
        .unwrap_or(0)
        .hash(&mut hasher);
    m::text(state, "isolate_seed").hash(&mut hasher);
    graph_viewport::hash_json_value(&state["kinds_off"], &mut hasher);
    let nodes = m::array(&doc["nodes"]);
    nodes.len().hash(&mut hasher);
    for n in &nodes {
        let id = node_id(n);
        id.hash(&mut hasher);
        m::text(n, "name").hash(&mut hasher);
        n["kind"].as_str().unwrap_or("").hash(&mut hasher);
        if let Some((px, py)) = pins.get(&id) {
            px.to_bits().hash(&mut hasher);
            py.to_bits().hash(&mut hasher);
        }
    }
    let edges = m::array(&doc["edges"]);
    edges.len().hash(&mut hasher);
    for e in &edges {
        m::text(e, "from").hash(&mut hasher);
        m::text(e, "to").hash(&mut hasher);
        primary_edge_kind(e).hash(&mut hasher);
        m::yes(&e["broken_risk"]).hash(&mut hasher);
        m::yes(&e["against"]).hash(&mut hasher);
        m::text(e, "provenance").hash(&mut hasher);
    }
    hasher.finish()
}

pub(super) fn filtered_doc(doc: &V, state: &V) -> V {
    let certain = m::yes(&state["certain"]);
    let filter = m::text(state, "filter").trim().to_ascii_lowercase();
    let isolate_active = m::yes(&state["isolate_active"]);
    if !certain && filter.is_empty() && !isolate_active {
        return doc.clone();
    }

    let mut nodes = m::array(&doc["nodes"]);
    let mut edges = m::array(&doc["edges"]);

    if certain {
        edges.retain(|e| {
            let prov = m::text(e, "provenance");
            let cert = m::text(e, "certainty");
            (cert.is_empty() || cert == "EXTRACTED") && prov != "guessed" && !m::yes(&e["guessed"])
        });
    }

    if !filter.is_empty() {
        let mut matched_ids = BTreeSet::new();
        for n in &nodes {
            let hay = format!(
                "{} {} {} {}",
                node_id(n),
                m::text(n, "name"),
                m::text(n, "role"),
                m::text(n, "language")
            )
            .to_ascii_lowercase();
            let contract_match = m::array(&n["provides"])
                .iter()
                .chain(m::array(&n["consumes"]).iter())
                .any(|c| contract_name(c).to_ascii_lowercase().contains(&filter));
            if hay.contains(&filter) || contract_match {
                matched_ids.insert(node_id(n));
                let p = m::text(n, "parent_service");
                if !p.is_empty() {
                    matched_ids.insert(p);
                }
            }
        }
        nodes.retain(|n| matched_ids.contains(&node_id(n)));
        edges.retain(|e| {
            matched_ids.contains(&m::text(e, "from")) && matched_ids.contains(&m::text(e, "to"))
        });
    }

    if isolate_active {
        let seed_id = {
            let s = m::text(state, "isolate_seed");
            if s.is_empty() {
                m::text(state, "focus")
            } else {
                s
            }
        };
        if !seed_id.is_empty() {
            let id_to_idx: BTreeMap<String, usize> = nodes
                .iter()
                .enumerate()
                .map(|(i, n)| (node_id(n), i))
                .collect();
            if let Some(&seed_idx) = id_to_idx.get(&seed_id) {
                let mut out_adj = vec![Vec::new(); nodes.len()];
                let mut in_adj = vec![Vec::new(); nodes.len()];
                for e in &edges {
                    if let (Some(&u), Some(&v)) = (
                        id_to_idx.get(&m::text(e, "from")),
                        id_to_idx.get(&m::text(e, "to")),
                    ) {
                        out_adj[u].push(v);
                        in_adj[v].push(u);
                    }
                }
                let depth = state["isolate_depth"].as_u64().unwrap_or(1).clamp(1, 3) as usize;
                let dist = graph_viewport::hop_cone_distances(
                    nodes.len(),
                    &out_adj,
                    &in_adj,
                    &[seed_idx],
                    depth,
                );
                let mut kept: BTreeSet<String> = nodes
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| dist[*i].is_some())
                    .map(|(_, n)| node_id(n))
                    .collect();
                // Keep children of kept services so unfolded frames remain intact.
                for n in &nodes {
                    let parent = first_non_empty(n, &["parent_service", "parent", "service"]);
                    if !parent.is_empty() && kept.contains(&parent) {
                        kept.insert(node_id(n));
                    }
                }
                nodes.retain(|n| kept.contains(&node_id(n)));
                edges.retain(|e| {
                    kept.contains(&m::text(e, "from")) && kept.contains(&m::text(e, "to"))
                });
            }
        }
    }

    let mut out = doc.clone();
    out["nodes"] = json!(nodes);
    out["edges"] = json!(edges);
    out
}
