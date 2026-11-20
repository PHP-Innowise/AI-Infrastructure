use super::*;

impl Domain for Graphs {
    fn emits_graph_status_events(&self) -> bool {
        true
    }

    fn execute<'a>(&'a self, operation: &'a str, request: Value) -> Reply<'a> {
        Box::pin(async move {
            let project = request["params"]["project_id"].as_str().unwrap_or("");
            let value = match operation {
                "graph.status" => {
                    let visit = match request["query"]["check"].as_str() {
                        None | Some("0") => false,
                        Some("1") => true,
                        Some(_) => return Err(HarnessError::invalid("Invalid graph check.")),
                    };
                    let me = self.me.upgrade().ok_or_else(|| {
                        HarnessError::unavailable("The code graph is shutting down.")
                    })?;
                    let project = project.to_owned();
                    tokio::task::spawn_blocking(move || me.status(&project, visit))
                        .await
                        .map_err(|_| {
                            HarnessError::unavailable("The code graph status could not be read.")
                        })??
                }
                "graph.auto_start" => {
                    if !project.is_empty()
                        && let Some(me) = self.me.upgrade()
                    {
                        let project = project.to_owned();
                        tokio::task::spawn_blocking(move || me.auto_start(&project))
                            .await
                            .map_err(|_| {
                                HarnessError::unavailable("The code graph could not start.")
                            })??;
                    }
                    json!({"ok": true})
                }
                "graph.auto_start_all" => {
                    if let Some(me) = self.me.upgrade() {
                        me.auto_start_projects();
                    }
                    json!({"ok": true})
                }
                "graph.act" => {
                    let body = &request["body"];
                    if !store::exact(body, &["action"], &["target"]) {
                        return Err(HarnessError::invalid("Invalid code graph action."));
                    }
                    match (body["action"].as_str().unwrap_or(""), body.get("target")) {
                        ("changes", Some(target)) => {
                            let target = Target::parse(target.as_str().unwrap_or(""))?;
                            let me = self.me.upgrade().ok_or_else(|| {
                                HarnessError::unavailable("The code graph is shutting down.")
                            })?;
                            let project = project.to_owned();
                            tokio::task::spawn_blocking(move || me.choose(&project, &target))
                                .await
                                .map_err(|_| {
                                    HarnessError::unavailable("The change could not be read.")
                                })??
                        }
                        (action, None) if action != "changes" => {
                            let me = self.me.upgrade().ok_or_else(|| {
                                HarnessError::unavailable("The code graph is shutting down.")
                            })?;
                            let project = project.to_owned();
                            let action = action.to_owned();
                            tokio::task::spawn_blocking(move || me.act(&project, &action))
                                .await
                                .map_err(|_| {
                                    HarnessError::unavailable(
                                        "The code graph action could not run.",
                                    )
                                })??
                        }
                        _ => return Err(HarnessError::invalid("Invalid code graph action.")),
                    }
                }
                "graph.pulls" | "graph.commits" | "graph.branches" => {
                    let query = &request["query"];
                    // What a person typed into a search: short, one line.
                    let typed = |name: &str| {
                        let value = query[name].as_str().unwrap_or("");
                        (value.len() <= 200 && !value.chars().any(char::is_control))
                            .then(|| value.to_owned())
                            .ok_or_else(|| HarnessError::invalid("Invalid search."))
                    };
                    let (state, wanted, scope) =
                        (typed("state")?, typed("query")?, typed("scope")?);
                    if !["", "open", "closed", "merged", "all"].contains(&state.as_str())
                        || scope.starts_with('-')
                        || scope.chars().any(char::is_whitespace)
                    {
                        return Err(HarnessError::invalid("Invalid search."));
                    }
                    let skip = match query["skip"].as_str() {
                        None | Some("") => 0,
                        Some(text) => text
                            .parse::<usize>()
                            .ok()
                            .filter(|skip| *skip <= 1_000_000)
                            .ok_or_else(|| HarnessError::invalid("Invalid search."))?,
                    };
                    let me = self.me.upgrade().ok_or_else(|| {
                        HarnessError::unavailable("The code graph is shutting down.")
                    })?;
                    let (project, operation) = (project.to_owned(), operation.to_owned());
                    tokio::task::spawn_blocking(move || match operation.as_str() {
                        "graph.commits" => me.commits(&project, &scope, &wanted, skip),
                        "graph.branches" => me.branches(&project),
                        // The open requests, unsearched, are the list the
                        // checkout's own work is measured with.
                        _ if wanted.is_empty() && ["", "open"].contains(&state.as_str()) => {
                            me.pulls(&project)
                        }
                        _ => me.found(&project, &state, &wanted),
                    })
                    .await
                    .map_err(|_| HarnessError::unavailable("The list could not be read."))??
                }
                "graph.map" => {
                    let query = &request["query"];
                    let text = |name: &str| {
                        let value = query[name].as_str().unwrap_or("");
                        (value.len() <= 1024)
                            .then(|| value.to_owned())
                            .ok_or_else(|| HarnessError::invalid("Invalid code graph place."))
                    };
                    let (scope, focus, filter) = (text("scope")?, text("focus")?, text("filter")?);
                    let focus_session = text("focus_session")?;
                    if focus_session.chars().any(char::is_control) {
                        return Err(HarnessError::invalid("Invalid code graph session."));
                    }
                    let flag = |name: &str| match query[name].as_str() {
                        None | Some("0") => Ok(false),
                        Some("1") => Ok(true),
                        Some(_) => Err(HarnessError::invalid("Invalid graph filter.")),
                    };
                    let (certain, notests) = (flag("certain")?, flag("notests")?);
                    let patterns = flag("patterns")?;
                    let history = flag("history")?;
                    let modules = flag("modules")?;
                    let no_incoming = flag("no_incoming")?;
                    let whole = flag("whole")?;
                    let origin = text("scenario")?;
                    let launch = text("launch")?;
                    let compare_launch = text("compare_launch")?;
                    let agent = text("agent")?;
                    if agent.chars().any(char::is_control) {
                        return Err(HarnessError::invalid("Invalid code graph agent."));
                    }
                    let target = query["target"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(Target::parse)
                        .transpose()?;
                    let followed = Followed::parse(
                        &text("follow")?,
                        query["out"].as_str().unwrap_or(""),
                        query["into"].as_str().unwrap_or(""),
                    )?;
                    let spread = places(
                        query["spread"].as_str().unwrap_or(""),
                        "Too many folders are open here; close some first.",
                        "Invalid folders to open here.",
                    )?;
                    // A link is named by its two ends, and by both.
                    let (from, to) = (text("from")?, text("to")?);
                    if from.is_empty() != to.is_empty() {
                        return Err(HarnessError::invalid("Invalid code graph link."));
                    }
                    // So is a path: a list of the place it leads from and
                    // the place it leads to.
                    let ends = places(
                        query["path"].as_str().unwrap_or(""),
                        "Invalid code graph path.",
                        "Invalid code graph path.",
                    )?;
                    if !ends.is_empty() && ends.len() != 2 {
                        return Err(HarnessError::invalid("Invalid code graph path."));
                    }
                    // A change may be told which tests fail: as many as a
                    // picture takes, each a name, a path or a line of the
                    // output of a test run.
                    let mut failing = places(
                        query["failing"].as_str().unwrap_or(""),
                        "Too many failing tests are named here.",
                        "Invalid failing tests.",
                    )?;
                    if failing.iter().any(|told| {
                        told.chars()
                            .any(|letter| letter.is_control() && letter != '\t')
                    }) {
                        return Err(HarnessError::invalid("Invalid failing tests."));
                    }
                    failing.truncate(crate::graph_map::FAILING);
                    // A path may ask for the ways that are longer than its
                    // shortest by a few links; nothing else may.
                    let longer = match query["longer"].as_str() {
                        None | Some("") => 0,
                        Some(text) if !ends.is_empty() => text
                            .parse::<usize>()
                            .ok()
                            .filter(|longer| *longer <= crate::graph_map::LONGER)
                            .ok_or_else(|| HarnessError::invalid("Invalid code graph path."))?,
                        Some(_) => {
                            return Err(HarnessError::invalid("Invalid code graph path."));
                        }
                    };
                    // And what two places share: a list of the two, asked
                    // for instead of the path between them.
                    let sharing = places(
                        query["shared"].as_str().unwrap_or(""),
                        "Invalid code graph places.",
                        "Invalid code graph places.",
                    )?;
                    if !sharing.is_empty() && (sharing.len() != 2 || !ends.is_empty()) {
                        return Err(HarnessError::invalid("Invalid code graph places."));
                    }
                    let cycle = text("cycle")?;
                    let base = text("base")?;
                    if !base.is_empty() {
                        let valid_hex = matches!(base.len(), 40 | 64)
                            && base.bytes().all(|byte| byte.is_ascii_hexdigit());
                        let valid_range_ref = matches!(target, Some(Target::Range { .. }))
                            && graph_changes::is_valid_range_ref(&base);
                        if !valid_hex && !valid_range_ref {
                            return Err(HarnessError::invalid("Invalid pull request base."));
                        }
                    }
                    if !cycle.is_empty()
                        && (cycle.chars().any(char::is_control)
                            || !from.is_empty()
                            || !ends.is_empty()
                            || !sharing.is_empty()
                            || followed.is_some())
                    {
                        return Err(HarnessError::invalid("Invalid code graph cycle."));
                    }
                    // One found pattern is asked for by itself.
                    let pattern = text("pattern")?;
                    if !pattern.is_empty()
                        && (pattern.chars().any(char::is_control)
                            || !from.is_empty()
                            || !ends.is_empty()
                            || !sharing.is_empty()
                            || !cycle.is_empty()
                            || followed.is_some())
                    {
                        return Err(HarnessError::invalid("Invalid code graph pattern."));
                    }
                    if !origin.is_empty()
                        && (modules
                            || !base.is_empty()
                            || !from.is_empty()
                            || !ends.is_empty()
                            || !sharing.is_empty()
                            || !cycle.is_empty()
                            || !pattern.is_empty()
                            || followed.is_some())
                    {
                        return Err(HarnessError::invalid("Invalid scenario view."));
                    }
                    // Of what they share one kind is drawn, and a request
                    // may say which.
                    let users = match query["side"].as_str() {
                        None | Some("") => None,
                        Some("both_use") if !sharing.is_empty() => Some(false),
                        Some("uses_both") if !sharing.is_empty() => Some(true),
                        Some(_) => {
                            return Err(HarnessError::invalid("Invalid code graph places."));
                        }
                    };
                    // The picture begins after as many of the nearest as
                    // a request says.
                    let skip = match query["skip"].as_str() {
                        None | Some("") => 0,
                        Some(text) if !sharing.is_empty() || !origin.is_empty() => text
                            .parse::<usize>()
                            .ok()
                            .filter(|skip| *skip <= 1_000_000)
                            .ok_or_else(|| HarnessError::invalid("Invalid code graph places."))?,
                        Some(_) => {
                            return Err(HarnessError::invalid("Invalid code graph places."));
                        }
                    };
                    let map_params = MapQueryParams::parse_with(
                        query["offset"].as_str(),
                        query["limit"].as_str(),
                        query["depth"].as_str(),
                        query["lod"].as_str(),
                        query["min_weight"].as_str(),
                    )?;
                    let me = self.me.upgrade().ok_or_else(|| {
                        HarnessError::unavailable("The code graph is shutting down.")
                    })?;
                    let project = project.to_owned();
                    tokio::task::spawn_blocking(move || {
                        me.map(
                            &project,
                            target.as_ref(),
                            &Shown {
                                scope: &scope,
                                focus: (!focus.is_empty()).then_some(focus.as_str()),
                                focus_session: (!focus_session.is_empty())
                                    .then_some(focus_session.as_str()),
                                certain,
                                filter: &filter,
                                followed: followed.as_ref(),
                                notests,
                                spread: &spread,
                                link: (!from.is_empty()).then_some((from.as_str(), to.as_str())),
                                path: match ends.as_slice() {
                                    [from, to] => Some((from.as_str(), to.as_str())),
                                    _ => None,
                                },
                                longer,
                                failing: &failing,
                                cycle: (!cycle.is_empty()).then_some(cycle.as_str()),
                                base: (!base.is_empty()).then_some(base.as_str()),
                                shared: match sharing.as_slice() {
                                    [from, to] => Some(Sharing {
                                        from,
                                        to,
                                        users,
                                        skip,
                                    }),
                                    _ => None,
                                },
                                patterns,
                                history,
                                modules,
                                no_incoming,
                                launch: (!launch.is_empty()).then_some(launch.as_str()),
                                compare_launch: (!compare_launch.is_empty())
                                    .then_some(compare_launch.as_str()),
                                agent: (!agent.is_empty()).then_some(agent.as_str()),
                                scenario: (!origin.is_empty()).then_some((origin.as_str(), skip)),
                                pattern: (!pattern.is_empty()).then_some(pattern.as_str()),
                                whole,
                                offset: map_params.offset,
                                limit: map_params.limit,
                                depth: map_params.depth,
                                lod: map_params.lod,
                                min_weight: map_params.min_weight,
                            },
                        )
                    })
                    .await
                    .map_err(|_| {
                        HarnessError::unavailable("The code graph map could not be read.")
                    })??
                }
                "graph.find" => {
                    let query = &request["query"];
                    // What a person typed into the field: one line of a
                    // hundred letters at most, of any script.
                    let wanted = query["query"].as_str().unwrap_or("");
                    if wanted.len() > 400 || wanted.chars().any(char::is_control) {
                        return Err(HarnessError::invalid("Invalid search."));
                    }
                    let notests = match query["notests"].as_str() {
                        None | Some("0") => false,
                        Some("1") => true,
                        Some(_) => return Err(HarnessError::invalid("Invalid graph filter.")),
                    };
                    let target = query["target"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(Target::parse)
                        .transpose()?;
                    let me = self.me.upgrade().ok_or_else(|| {
                        HarnessError::unavailable("The code graph is shutting down.")
                    })?;
                    let (project, wanted) = (project.to_owned(), wanted.to_owned());
                    tokio::task::spawn_blocking(move || {
                        me.find(&project, target.as_ref(), &wanted, notests)
                    })
                    .await
                    .map_err(|_| {
                        HarnessError::unavailable("The code graph could not be searched.")
                    })??
                }
                "graph.changes" | "graph.file_impact" => {
                    let query = &request["query"];
                    let flag = |name: &str| match query[name].as_str() {
                        None | Some("0") => Ok(false),
                        Some("1") => Ok(true),
                        Some(_) => Err(HarnessError::invalid("Invalid graph filter.")),
                    };
                    let is_impact = operation == "graph.file_impact";
                    let raw_target = query["target"].as_str().unwrap_or("");
                    let raw_from = query["from"].as_str().unwrap_or("");
                    let raw_to = query["to"].as_str().unwrap_or("");
                    let target = if !raw_target.is_empty() {
                        if !raw_from.is_empty() || !raw_to.is_empty() {
                            return Err(HarnessError::invalid(
                                "Specify either target or from/to range parameters, not both.",
                            ));
                        }
                        Some(Target::parse(raw_target)?)
                    } else if !raw_from.is_empty() || !raw_to.is_empty() {
                        if raw_from.is_empty() || raw_to.is_empty() {
                            return Err(HarnessError::invalid(
                                "Both 'from' and 'to' are required for a range.",
                            ));
                        }
                        Some(Target::parse(&format!("range:{raw_from}..{raw_to}"))?)
                    } else {
                        None
                    };
                    let focus_session = query["focus_session"].as_str().unwrap_or("");
                    if focus_session.len() > 1024 || focus_session.chars().any(char::is_control) {
                        return Err(HarnessError::invalid("Invalid code graph session."));
                    }
                    let focus_session =
                        (!focus_session.trim().is_empty()).then(|| focus_session.trim().to_owned());
                    let agent = query["agent"].as_str().unwrap_or("");
                    if agent.len() > 256 || agent.chars().any(char::is_control) {
                        return Err(HarnessError::invalid("Invalid code graph agent."));
                    }
                    let agent = (!agent.trim().is_empty()).then(|| agent.trim().to_owned());
                    let path = if is_impact {
                        let value = query["path"].as_str().unwrap_or("");
                        if value.trim().is_empty()
                            || value.len() > 1024
                            || value.chars().any(char::is_control)
                        {
                            return Err(HarnessError::invalid("Invalid code graph file path."));
                        }
                        value.to_owned()
                    } else {
                        String::new()
                    };
                    let toggle = if is_impact {
                        flag("certain")?
                    } else {
                        flag("notests")?
                    };
                    let me = self.me.upgrade().ok_or_else(|| {
                        HarnessError::unavailable("The code graph is shutting down.")
                    })?;
                    let project = project.to_owned();
                    tokio::task::spawn_blocking(move || {
                        if is_impact {
                            me.file_impact_with_agent(
                                &project,
                                &path,
                                target.as_ref(),
                                focus_session.as_deref(),
                                agent.as_deref(),
                                toggle,
                            )
                        } else {
                            me.changes_catalog_with_agent(
                                &project,
                                target.as_ref(),
                                focus_session.as_deref(),
                                agent.as_deref(),
                                toggle,
                            )
                        }
                    })
                    .await
                    .map_err(|_| {
                        HarnessError::unavailable("The code graph could not be inspected.")
                    })??
                }
                _ => return Err(HarnessError::missing("Unknown code graph operation.")),
            };
            Ok(Payload::Json(value))
        })
    }
}
