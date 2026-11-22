//! What a change to a place reaches.
//!
//! The names that use a changed place may notice the change, and so may
//! the names that use those, a few links on. Each link passes on less than
//! it received: a call passes on most, an import alone little, a link the
//! tool only guessed less again. So a place is told with how many links
//! away it is and with how much of the change is left there, and the list
//! reads from what will notice to what might.
//!
//! A change also reaches past what the changed type fulfils. Where a type
//! implements an interface or extends a base, the names that use the
//! interface never name the type, yet they run its code. They are reached
//! through the abstraction, and only a part of the change is left there: a
//! caller of an interface notices a change of one implementation only where
//! the promise of the interface changes with it.

use super::*;

/// How many links from the changed place are walked.
pub(crate) const BLAST_HOPS: usize = 4;
/// How many files are listed, and for how many a picture is told the hop.
const LISTED: usize = 12;
const MARKED: usize = 400;
/// How many links a walk looks at before it gives up.
const WALKED: usize = 400_000;
/// Less than this of a change is not worth telling.
const FLOOR: f64 = 0.04;
/// What is left of a change that passes what a type fulfils.
const THROUGH_ABSTRACTION: f64 = 0.4;
/// What is left of it along a link the tool only guessed.
const GUESSED: f64 = 0.6;

/// How much of a change a link of a relation passes on to what uses the
/// changed place, and the kind of arrow the relation is drawn as.
fn carried(relation: &str) -> (f64, &'static str) {
    match relation {
        "calls" | "dispatches_to" | "instantiates" | "indirect_call" => (0.85, "calls"),
        "inherits" | "extends" => (0.9, "extends"),
        "implements" | "uses_trait" => (0.9, "implements"),
        "writes" | "mutates" | "sets" => (0.75, "writes"),
        "reads" | "references_constant" | "uses_const" | "uses_static_prop" => (0.7, "reads"),
        "references" => (0.75, "references"),
        "di_binds" | "di_binding" => (0.75, "di_binds"),
        "calls_route" => (0.75, "calls_route"),
        "exposes_route" | "route" => (0.7, "exposes_route"),
        "publishes_event" => (0.7, "publishes_event"),
        "handles_event" => (0.7, "handles_event"),
        "writes_table" | "table_write" => (0.7, "writes_table"),
        "reads_table" | "table_read" => (0.65, "reads_table"),
        "imports" | "imports_from" | "dynamic_import" | "re_exports" => (0.45, "imports"),
        _ => (0.6, "uses"),
    }
}

fn enters_semantic_anchor(relation: &str) -> bool {
    matches!(
        relation,
        "writes_table"
            | "table_write"
            | "publishes_event"
            | "exposes_route"
            | "route"
            | "di_binds"
            | "di_binding"
    )
}

/// A relation by which a type says what it fulfils.
fn fulfils(relation: &str) -> bool {
    matches!(
        relation,
        "inherits" | "extends" | "implements" | "uses_trait"
    )
}

#[derive(Clone, Copy)]
struct Reached {
    hop: u8,
    left: f64,
    /// The kind of the last link that led here.
    via: &'static str,
    /// The way here passed what a type fulfils.
    through: bool,
}

impl Index {
    /// What a change to `place` reaches within `BLAST_HOPS` links: a file,
    /// a folder, or a symbol named as the pictures name it. `certain`
    /// leaves out the links the tool inferred. The answer counts the names
    /// and files reached by hop, lists the files most of the change is
    /// left in, and tells for up to `MARKED` files the hop and what is
    /// left, for a picture to mark its boxes with. Tests are told apart:
    /// they are what would notice first. `None` where the place names
    /// nothing in the graph.
    pub(crate) fn blast(&self, place: &str, certain: bool) -> Option<Value> {
        let reach = self.reach(place)?;
        let mut starts = match reach {
            Reach::Symbol(at) => vec![at],
            Reach::File(file) => self.range(file).collect(),
            Reach::Folder(folder) => self
                .files
                .iter()
                .enumerate()
                .filter(|(_, file)| {
                    file.strip_prefix(folder)
                        .is_some_and(|rest| rest.starts_with('/'))
                })
                .flat_map(|(file, _)| self.range(file as u32))
                .collect(),
        };
        if !matches!(reach, Reach::Symbol(_)) {
            let non_semantic: Vec<u32> = starts
                .iter()
                .copied()
                .filter(|&at| !self.symbols[at as usize].is_semantic)
                .collect();
            if !non_semantic.is_empty() {
                starts = non_semantic;
            }
        }
        let own = starts
            .iter()
            .map(|at| self.symbols[*at as usize].file)
            .collect::<HashSet<_>>();
        let mut best: HashMap<u32, Reached> = starts
            .iter()
            .map(|at| {
                (
                    *at,
                    Reached {
                        hop: 0,
                        left: 1.0,
                        via: "",
                        through: false,
                    },
                )
            })
            .collect();
        let mut front = starts;
        let mut walked = 0usize;
        let mut cut = false;
        'hops: for hop in 1..=BLAST_HOPS as u8 {
            // What a hop reaches, or reaches with more of the change left
            // than before, passes that on at the next.
            let mut next = Vec::new();
            let mut queued = HashSet::new();
            for at in std::mem::take(&mut front) {
                let from = best[&at];
                // Who uses it.
                for (user, _, kind, guessed) in self.wired(at, false, certain) {
                    walked += 1;
                    if walked > WALKED {
                        cut = true;
                        break 'hops;
                    }
                    if self.symbols[*user as usize].kind == Kind::Note {
                        continue;
                    }
                    let (carries, via) = carried(&self.kinds[*kind as usize]);
                    let left = from.left * carries * if *guessed { GUESSED } else { 1.0 };
                    if left < FLOOR {
                        continue;
                    }
                    let known = best.get(user).map_or(0.0, |known| known.left);
                    if left > known {
                        if queued.insert(*user) {
                            next.push(*user);
                        }
                        let hop = best.get(user).map_or(hop, |known| known.hop.min(hop));
                        best.insert(
                            *user,
                            Reached {
                                hop,
                                left,
                                via,
                                through: from.through,
                            },
                        );
                    }
                }
                // What it fulfils or delegates to via semantic anchors and direct DI bindings.
                let symbol = &self.symbols[at as usize];
                if !symbol.is_semantic {
                    for (_, target, kind, guessed) in self.wired(at, true, certain) {
                        let rel = &self.kinds[*kind as usize];
                        let is_direct_di_impl = matches!(rel.as_str(), "di_binds" | "di_binding");
                        if !self.symbols[*target as usize].is_semantic && !is_direct_di_impl {
                            continue;
                        }
                        if !enters_semantic_anchor(rel) {
                            continue;
                        }
                        walked += 1;
                        if walked > WALKED {
                            cut = true;
                            break 'hops;
                        }
                        let (carries, via) = carried(rel);
                        let left = from.left * carries * if *guessed { GUESSED } else { 1.0 };
                        if left < FLOOR || best.get(target).is_some_and(|known| known.left >= left)
                        {
                            continue;
                        }
                        if queued.insert(*target) {
                            next.push(*target);
                        }
                        let hop = best.get(target).map_or(hop, |known| known.hop.min(hop));
                        best.insert(
                            *target,
                            Reached {
                                hop,
                                left,
                                via,
                                through: from.through,
                            },
                        );
                    }
                }
                let holder = match symbol.kind {
                    Kind::Note | Kind::Heading | Kind::File => continue,
                    Kind::Type => at,
                    _ if symbol.parent != NONE => symbol.parent,
                    _ => at,
                };
                for (_, fulfilled, kind, guessed) in self.wired(holder, true, certain) {
                    walked += 1;
                    if walked > WALKED {
                        cut = true;
                        break 'hops;
                    }
                    if !fulfils(&self.kinds[*kind as usize]) {
                        continue;
                    }
                    let left =
                        from.left * THROUGH_ABSTRACTION * if *guessed { GUESSED } else { 1.0 };
                    if left < FLOOR || best.get(fulfilled).is_some_and(|known| known.left >= left) {
                        continue;
                    }
                    if queued.insert(*fulfilled) {
                        next.push(*fulfilled);
                    }
                    let hop = best.get(fulfilled).map_or(hop, |known| known.hop.min(hop));
                    best.insert(
                        *fulfilled,
                        Reached {
                            hop,
                            left,
                            via: carried(&self.kinds[*kind as usize]).1,
                            through: true,
                        },
                    );
                }
            }
            if next.is_empty() {
                break;
            }
            front = next;
        }
        // By file: the name of it most of the change is left in, and the
        // fewest links to any name of it.
        let mut files: HashMap<u32, (u32, Reached, usize)> = HashMap::new();
        // Of each hop, how many files are first reached there, and how many
        // names are.
        let mut by_hop = [(0usize, 0usize); BLAST_HOPS + 1];
        for (at, reached) in &best {
            let file = self.symbols[*at as usize].file;
            if reached.hop == 0 || own.contains(&file) {
                continue;
            }
            by_hop[reached.hop as usize].1 += 1;
            let entry = files.entry(file).or_insert((*at, *reached, 0));
            entry.2 += 1;
            let better = reached.left > entry.1.left
                || (reached.left == entry.1.left && self.tag_of(*at) < self.tag_of(entry.0));
            if better {
                entry.0 = *at;
                entry.1 = Reached {
                    hop: reached.hop.min(entry.1.hop),
                    ..*reached
                };
            } else {
                entry.1.hop = entry.1.hop.min(reached.hop);
            }
        }
        for (_, reached, _) in files.values() {
            by_hop[reached.hop as usize].0 += 1;
        }
        let mut ordered = files.into_iter().collect::<Vec<_>>();
        ordered.sort_by(|(a, left), (b, right)| {
            right
                .1
                .left
                .total_cmp(&left.1.left)
                .then(left.1.hop.cmp(&right.1.hop))
                .then(self.files[*a as usize].cmp(&self.files[*b as usize]))
        });
        // To two places; what stands halfway between two is told as the
        // larger, whatever the last bit of the product says.
        let rounded = |left: f64| (left * 100.0 + 1e-6).round() / 100.0;
        let tests = ordered
            .iter()
            .filter(|(file, _)| is_test(&self.files[*file as usize]))
            .count();
        let listed = ordered
            .iter()
            .take(LISTED)
            .map(|(file, (at, reached, names))| {
                json!({
                    "path": self.files[*file as usize],
                    "hop": reached.hop,
                    "left": rounded(reached.left),
                    "name": self.shown(*at),
                    "via": reached.via,
                    "through": reached.through,
                    "names": names,
                    "test": is_test(&self.files[*file as usize]),
                })
            })
            .collect::<Vec<_>>();
        let marked = ordered
            .iter()
            .take(MARKED)
            .map(|(file, (_, reached, _))| {
                json!([
                    self.files[*file as usize],
                    reached.hop,
                    rounded(reached.left)
                ])
            })
            .collect::<Vec<_>>();
        Some(json!({
            "place": place,
            "hops": BLAST_HOPS,
            "files": ordered.len(),
            "names": by_hop.iter().map(|(_, names)| names).sum::<usize>(),
            "tests": tests,
            "cut": cut,
            "by_hop": by_hop
                .iter()
                .enumerate()
                .skip(1)
                .map(|(hop, (files, names))| json!({"hop": hop, "files": files, "names": names}))
                .collect::<Vec<_>>(),
            "listed": listed,
            "more": ordered.len().saturating_sub(LISTED),
            "marked": marked,
        }))
    }

    /// Tells the selected place of a picture what a change to it reaches,
    /// by the same links the picture is drawn by.
    pub(crate) fn mark_blast(&self, view: &mut Value) {
        let Some(place) = view["focus"]["path"]
            .as_str()
            .filter(|path| !path.is_empty())
        else {
            return;
        };
        if let Some(blast) = self.blast(place, view["certain"] == true) {
            view["focus"]["blast"] = blast;
        }
    }

    /// Where a symbol lies, to tell two apart that are as much reached.
    fn tag_of(&self, at: u32) -> (u32, u32, u32) {
        let symbol = &self.symbols[at as usize];
        (symbol.file, symbol.line, at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(links: &[(&str, &str, &str, &str)]) -> Index {
        // Every name is `file:name`; a name that starts with a capital is a
        // type, any other a function.
        let mut names = links
            .iter()
            .flat_map(|(from, to, ..)| [*from, *to])
            .collect::<Vec<_>>();
        names.sort_unstable();
        names.dedup();
        let nodes = names
            .iter()
            .map(|id| {
                let (file, name) = id.split_once(':').unwrap();
                let label = if name.chars().next().is_some_and(char::is_uppercase) {
                    name.to_owned()
                } else {
                    format!("{name}()")
                };
                json!({
                    "id": id, "label": label, "source_file": file, "source_location": "L2",
                    "_callable_class": if label.ends_with("()") { "false" } else { "true" },
                })
            })
            .collect::<Vec<_>>();
        let links = links
            .iter()
            .map(|(from, to, relation, confidence)| {
                json!({"source": from, "target": to, "relation": relation, "confidence": confidence})
            })
            .collect::<Vec<_>>();
        Index::read(
            json!({"nodes": nodes, "links": links})
                .to_string()
                .as_bytes(),
            Path::new("/work/shop"),
        )
        .unwrap()
    }

    fn told(blast: &Value) -> Vec<String> {
        blast["listed"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                format!(
                    "{} hop {} left {} via {}{}{}",
                    row["path"].as_str().unwrap(),
                    row["hop"],
                    row["left"],
                    row["via"].as_str().unwrap(),
                    if row["through"] == true {
                        " through"
                    } else {
                        ""
                    },
                    if row["test"] == true { " test" } else { "" },
                )
            })
            .collect()
    }

    const READ: &str = "EXTRACTED";

    #[test]
    fn a_change_is_passed_on_from_user_to_user_and_less_of_it_each_time() {
        // price is called by cart, cart by checkout, checkout by web, web
        // by boot, boot by main: five links, of which four are walked. An
        // import alone passes on less than a call, a guess less again.
        let index = project(&[
            ("src/cart.php:cart", "src/price.php:price", "calls", READ),
            (
                "src/checkout.php:checkout",
                "src/cart.php:cart",
                "calls",
                READ,
            ),
            (
                "src/web.php:web",
                "src/checkout.php:checkout",
                "calls",
                READ,
            ),
            ("src/boot.php:boot", "src/web.php:web", "calls", READ),
            ("src/main.php:main", "src/boot.php:boot", "calls", READ),
            (
                "src/report.php:report",
                "src/price.php:price",
                "imports",
                READ,
            ),
            (
                "src/guess.php:guess",
                "src/price.php:price",
                "calls",
                "INFERRED",
            ),
            (
                "tests/PriceTest.php:testPrice",
                "src/price.php:price",
                "calls",
                READ,
            ),
            ("src/price.php:price", "src/money.php:money", "calls", READ),
        ]);
        let blast = index.blast("src/price.php", false).unwrap();
        assert_eq!(
            told(&blast),
            [
                "src/cart.php hop 1 left 0.85 via calls",
                "tests/PriceTest.php hop 1 left 0.85 via calls test",
                "src/checkout.php hop 2 left 0.72 via calls",
                "src/web.php hop 3 left 0.61 via calls",
                "src/boot.php hop 4 left 0.52 via calls",
                "src/guess.php hop 1 left 0.51 via calls",
                "src/report.php hop 1 left 0.45 via imports",
            ]
        );
        // What the changed place uses is not reached, nor what lies a
        // fifth link away.
        assert_eq!(blast["files"], 7);
        assert_eq!(blast["tests"], 1);
        assert_eq!(blast["more"], 0);
        assert_eq!(blast["cut"], false);
        assert_eq!(
            blast["by_hop"],
            json!([
                {"hop": 1, "files": 4, "names": 4},
                {"hop": 2, "files": 1, "names": 1},
                {"hop": 3, "files": 1, "names": 1},
                {"hop": 4, "files": 1, "names": 1}
            ])
        );
        // A picture is told the hop of each file, and what is left there.
        assert_eq!(blast["marked"][0], json!(["src/cart.php", 1, 0.85]));
        assert_eq!(blast["marked"].as_array().unwrap().len(), 7);
        // Without the guessed links the guess is not reached.
        let certain = index.blast("src/price.php", true).unwrap();
        assert_eq!(certain["files"], 6);
        assert!(
            !told(&certain)
                .iter()
                .any(|row| row.starts_with("src/guess.php"))
        );
        // A symbol is a place of its own, and so is a folder: all of src
        // is reached by nothing but the test.
        let symbol = index.blast("src/cart.php#L2:cart()", false).unwrap();
        assert_eq!(symbol["files"], 4);
        assert_eq!(
            told(&index.blast("src", false).unwrap()),
            ["tests/PriceTest.php hop 1 left 0.85 via calls test"]
        );
        // A place the graph does not know reaches nothing that can be told.
        assert!(index.blast("src/gone.php", false).is_none());
    }

    #[test]
    fn a_change_reaches_past_what_its_type_fulfils_and_weaker() {
        // Stripe implements Gateway. Checkout calls the interface and
        // never names Stripe; Refund names Stripe itself. Billing calls
        // Checkout.
        let index = project(&[
            (
                "src/Stripe.php:Stripe",
                "src/Gateway.php:Gateway",
                "implements",
                READ,
            ),
            (
                "src/Checkout.php:Checkout",
                "src/Gateway.php:Gateway",
                "calls",
                READ,
            ),
            (
                "src/Refund.php:Refund",
                "src/Stripe.php:Stripe",
                "calls",
                READ,
            ),
            (
                "src/Billing.php:Billing",
                "src/Checkout.php:Checkout",
                "calls",
                READ,
            ),
        ]);
        let blast = index.blast("src/Stripe.php", false).unwrap();
        assert_eq!(
            told(&blast),
            [
                // Who names the type notices most.
                "src/Refund.php hop 1 left 0.85 via calls",
                // The interface it fulfils is reached, weakened, and
                // through it those who call the interface.
                "src/Gateway.php hop 1 left 0.4 via implements through",
                "src/Checkout.php hop 2 left 0.34 via calls through",
                "src/Billing.php hop 3 left 0.29 via calls through",
            ]
        );
        // A change to the interface itself reaches its implementation at
        // full strength: it has to follow.
        assert_eq!(
            told(&index.blast("src/Gateway.php", false).unwrap()),
            [
                "src/Stripe.php hop 1 left 0.9 via implements",
                "src/Checkout.php hop 1 left 0.85 via calls",
                "src/Refund.php hop 2 left 0.77 via calls",
                "src/Billing.php hop 2 left 0.72 via calls",
            ]
        );
    }

    #[test]
    fn a_file_is_counted_at_the_hop_of_its_nearest_name() {
        // a calls core directly; b calls a; a second name of b's file, c,
        // reaches core only through b: the file of b and c is one file,
        // first reached at the second link.
        let index = project(&[
            ("src/a.php:a", "src/core.php:core", "calls", READ),
            ("src/b.php:b", "src/a.php:a", "calls", READ),
            ("src/b.php:c", "src/b.php:b", "calls", READ),
        ]);
        let blast = index.blast("src/core.php", false).unwrap();
        assert_eq!(
            blast["by_hop"],
            json!([
                {"hop": 1, "files": 1, "names": 1},
                {"hop": 2, "files": 1, "names": 1},
                {"hop": 3, "files": 0, "names": 1},
                {"hop": 4, "files": 0, "names": 0}
            ])
        );
        assert_eq!(
            (blast["files"].clone(), blast["names"].clone()),
            (json!(2), json!(3))
        );
        // The row of b's file names the name most of the change is left in.
        assert_eq!(blast["listed"][1]["name"], "b()");
        assert_eq!(blast["listed"][1]["names"], 2);
    }

    #[test]
    fn a_picture_tells_its_selected_place_what_a_change_to_it_reaches() {
        let index = project(&[
            ("src/cart.php:cart", "src/price.php:price", "calls", READ),
            (
                "src/guess.php:guess",
                "src/price.php:price",
                "calls",
                "INFERRED",
            ),
        ]);
        let mut view = index.view("shop", "src", Some("src/price.php"), false, "");
        index.mark_blast(&mut view);
        assert_eq!(view["focus"]["blast"]["files"], 2);
        assert_eq!(view["focus"]["blast"]["place"], "src/price.php");
        // Among the links read from the code the guess reaches nothing.
        let mut certain = index.view("shop", "src", Some("src/price.php"), true, "");
        index.mark_blast(&mut certain);
        assert_eq!(certain["focus"]["blast"]["files"], 1);
        // Nothing selected, or what the graph does not know: nothing told.
        let mut plain = index.view("shop", "src", None, false, "");
        index.mark_blast(&mut plain);
        assert!(plain["focus"].is_null());
        let mut module = json!({"focus": {"path": "module:Billing"}});
        index.mark_blast(&mut module);
        assert!(module["focus"]["blast"].is_null());
    }

    #[test]
    fn many_files_are_counted_and_the_most_reached_are_listed() {
        // Thirty callers of one function.
        let callers = (0..30)
            .map(|at| format!("src/c{at:02}.php:c{at:02}"))
            .collect::<Vec<_>>();
        let links = callers
            .iter()
            .map(|caller| (caller.as_str(), "src/core.php:core", "calls", READ))
            .collect::<Vec<_>>();
        let blast = project(&links).blast("src/core.php", false).unwrap();
        assert_eq!(blast["files"], 30);
        assert_eq!(blast["listed"].as_array().unwrap().len(), LISTED);
        assert_eq!(blast["more"], 30 - LISTED);
        assert_eq!(blast["marked"].as_array().unwrap().len(), 30);
        // As much reached: in the order of their files.
        assert_eq!(blast["listed"][0]["path"], "src/c00.php");
    }

    #[test]
    fn blast_bridges_across_semantic_anchor_nodes() {
        let graph = json!({
            "nodes": [
                {"id": "writer", "label": "OrderRepo.save()", "source_file": "src/OrderRepo.php"},
                {"id": "reader", "label": "InvoiceReport.run()", "source_file": "src/InvoiceReport.php"},
                {"id": "sem:table:orders", "label": "table:orders", "source_file": "src/OrderRepo.php", "node_kind": "semantic"}
            ],
            "links": [
                {"source": "writer", "target": "sem:table:orders", "relation": "writes_table", "confidence": "EXTRACTED"},
                {"source": "reader", "target": "sem:table:orders", "relation": "reads_table", "confidence": "EXTRACTED"}
            ]
        });
        let idx = Index::read(graph.to_string().as_bytes(), Path::new("/work/shop")).unwrap();
        let blast = idx
            .blast("src/OrderRepo.php", true)
            .expect("semantic blast");
        assert_eq!(blast["files"], 1);
        assert_eq!(blast["listed"][0]["path"], "src/InvoiceReport.php");
        assert_eq!(blast["listed"][0]["hop"], 2);
        assert_eq!(blast["listed"][0]["via"], "reads_table");
    }

    #[test]
    fn blast_propagates_across_direct_di_binds_in_both_directions() {
        let graph = json!({
            "nodes": [
                {"id": "ctrl", "label": "OrderController", "source_file": "src/OrderController.php", "node_kind": "class"},
                {"id": "iface", "label": "GatewayInterface", "source_file": "src/GatewayInterface.php", "node_kind": "interface"},
                {"id": "impl", "label": "StripeGateway", "source_file": "src/StripeGateway.php", "node_kind": "class"}
            ],
            "links": [
                {"source": "ctrl", "target": "iface", "relation": "references", "confidence": "EXTRACTED"},
                {"source": "iface", "target": "impl", "relation": "di_binds", "confidence": "EXTRACTED"}
            ]
        });
        let idx = Index::read(graph.to_string().as_bytes(), Path::new("/work/shop")).unwrap();

        let from_impl = idx
            .blast("src/StripeGateway.php", true)
            .expect("impl blast");
        assert!(
            from_impl["listed"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["path"] == "src/OrderController.php"),
            "editing concrete impl should reach interface caller: {from_impl}"
        );

        let from_iface = idx
            .blast("src/GatewayInterface.php", true)
            .expect("iface blast");
        assert!(
            from_iface["listed"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["path"] == "src/StripeGateway.php"),
            "editing interface should reach DI-bound impl: {from_iface}"
        );
    }
}
