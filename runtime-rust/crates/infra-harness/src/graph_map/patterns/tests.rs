//! Tests of the patterns a graph shows signs of.
use super::*;

/// A graph put together type by type, as the tool writes one.
#[derive(Default)]
struct Graph {
    nodes: Vec<Value>,
    links: Vec<Value>,
    sources: HashMap<String, String>,
}
impl Graph {
    /// A type in a file of its own, declared on the given line of it.
    fn class(&mut self, name: &str, file: &str, declared: &str) -> &mut Self {
        self.nodes.push(json!({
            "id": name, "label": name, "source_file": file, "source_location": "L3",
            "_callable_class": true,
        }));
        self.sources
            .insert(file.to_owned(), format!("<?php\n\n{declared}\n{{\n}}\n"));
        self
    }
    /// A method of a type, named `Type.method` among the nodes.
    fn method(&mut self, class: &str, name: &str) -> &mut Self {
        let file =
            self.nodes.iter().find(|node| node["id"] == class).unwrap()["source_file"].clone();
        let line = 5 + self.nodes.len();
        self.nodes.push(json!({
            "id": format!("{class}.{name}"), "label": format!(".{name}()"),
            "source_file": file, "source_location": format!("L{line}"),
        }));
        self.links.push(
            json!({"source": class, "target": format!("{class}.{name}"), "relation": "method"}),
        );
        self
    }
    fn link(&mut self, from: &str, to: &str, relation: &str) -> &mut Self {
        self.links.push(
            json!({"source": from, "target": to, "relation": relation, "confidence": "EXTRACTED"}),
        );
        self
    }
    fn guessed(&mut self, from: &str, to: &str, relation: &str) -> &mut Self {
        self.links.push(
            json!({"source": from, "target": to, "relation": relation, "confidence": "INFERRED"}),
        );
        self
    }
    /// One symbol names a type as the type of something of its own.
    fn names(&mut self, from: &str, to: &str, context: &str) -> &mut Self {
        self.links.push(json!({
            "source": from, "target": to, "relation": "references", "context": context,
            "confidence": "EXTRACTED",
        }));
        self
    }
    fn index(&self) -> Index {
        let graph = json!({"nodes": self.nodes, "links": self.links});
        Index::read(graph.to_string().as_bytes(), Path::new("/work/shop")).unwrap()
    }
    fn found(&self, index: &Index, certain: bool) -> Patterns {
        Patterns::find(index, certain, &|path| self.sources.get(path).cloned())
    }
}
/// A shop with one of each pattern that is looked for.
fn shop() -> Graph {
    let mut graph = Graph::default();
    graph
        .class(
            "PaymentGateway",
            "app/Payments/PaymentGateway.php",
            "interface PaymentGateway",
        )
        .method("PaymentGateway", "charge")
        .class(
            "StripeGateway",
            "app/Payments/StripeGateway.php",
            "class StripeGateway implements PaymentGateway",
        )
        .method("StripeGateway", "charge")
        .class(
            "PaypalGateway",
            "app/Payments/PaypalGateway.php",
            "class PaypalGateway implements PaymentGateway",
        )
        .method("PaypalGateway", "charge")
        .class(
            "LoggingGateway",
            "app/Payments/LoggingGateway.php",
            "class LoggingGateway implements PaymentGateway",
        )
        .method("LoggingGateway", "__construct")
        .method("LoggingGateway", "charge")
        .link("StripeGateway", "PaymentGateway", "implements")
        .link("PaypalGateway", "PaymentGateway", "implements")
        .link("LoggingGateway", "PaymentGateway", "implements")
        .names("LoggingGateway", "PaymentGateway", "field")
        .names(
            "LoggingGateway.__construct",
            "PaymentGateway",
            "parameter_type",
        )
        .class(
            "GatewayFactory",
            "app/Payments/GatewayFactory.php",
            "class GatewayFactory",
        )
        .method("GatewayFactory", "make")
        .names("GatewayFactory.make", "PaymentGateway", "return_type")
        .link("GatewayFactory.make", "PaypalGateway", "calls")
        .guessed("GatewayFactory.make", "StripeGateway", "calls")
        .class("Logger", "app/Support/Logger.php", "class Logger")
        .method("Logger", "getInstance")
        .method("Logger", "write")
        .link("LoggingGateway.charge", "Logger.getInstance", "calls")
        .class("Report", "app/Support/Report.php", "class Report")
        .class(
            "ReportBuilder",
            "app/Support/ReportBuilder.php",
            "class ReportBuilder",
        )
        .method("ReportBuilder", "title")
        .method("ReportBuilder", "build")
        .names("ReportBuilder.build", "Report", "return_type")
        .class(
            "AbstractExporter",
            "app/Support/AbstractExporter.php",
            "abstract class AbstractExporter",
        )
        .method("AbstractExporter", "export")
        .method("AbstractExporter", "header")
        .method("AbstractExporter", "body")
        .link(
            "AbstractExporter.export",
            "AbstractExporter.header",
            "calls",
        )
        .link("AbstractExporter.export", "AbstractExporter.body", "calls")
        // A helper of the base that no type under it defines is no step.
        .method("AbstractExporter", "escape")
        .link(
            "AbstractExporter.export",
            "AbstractExporter.escape",
            "calls",
        )
        .class(
            "CsvExporter",
            "app/Support/CsvExporter.php",
            "class CsvExporter extends AbstractExporter",
        )
        .method("CsvExporter", "header")
        .method("CsvExporter", "body")
        .class(
            "HtmlExporter",
            "app/Support/HtmlExporter.php",
            "class HtmlExporter extends AbstractExporter",
        )
        .method("HtmlExporter", "header")
        .link("CsvExporter", "AbstractExporter", "inherits")
        .link("HtmlExporter", "AbstractExporter", "inherits")
        .class("Order", "app/Models/Order.php", "class Order")
        .class(
            "OrderRepository",
            "app/Repositories/OrderRepository.php",
            "class OrderRepository",
        )
        .method("OrderRepository", "find")
        .method("OrderRepository", "save")
        .names("OrderRepository.find", "Order", "return_type")
        .names("OrderRepository.save", "Order", "parameter_type")
        .class("OrderPaid", "app/Events/OrderPaid.php", "class OrderPaid")
        .class(
            "SendReceipt",
            "app/Listeners/SendReceipt.php",
            "class SendReceipt",
        )
        .method("SendReceipt", "handle")
        .names("SendReceipt.handle", "OrderPaid", "parameter_type")
        .class(
            "CheckoutService",
            "app/Services/CheckoutService.php",
            "class CheckoutService",
        )
        .method("CheckoutService", "__construct")
        .method("CheckoutService", "pay")
        .names("CheckoutService", "OrderRepository", "field")
        .names("CheckoutService", "PaymentGateway", "field")
        .names(
            "CheckoutService.__construct",
            "PaymentGateway",
            "parameter_type",
        )
        .link("CheckoutService.pay", "OrderPaid", "calls")
        .class(
            "RefundOrder",
            "app/Commands/RefundOrder.php",
            "class RefundOrder",
        )
        .class(
            "RefundOrderHandler",
            "app/Commands/RefundOrderHandler.php",
            "class RefundOrderHandler",
        )
        .method("RefundOrderHandler", "handle")
        .names("RefundOrderHandler.handle", "RefundOrder", "parameter_type")
        .names("RefundOrderHandler", "OrderRepository", "field");
    graph
}
/// A finding as its pattern, its anchor, and its other parts with their
/// roles.
fn told(index: &Index, found: &Found) -> (String, String, Vec<(String, String)>) {
    (
        SORTS[found.sort].id.to_owned(),
        index.shown(found.anchor),
        found.parts[1..]
            .iter()
            .map(|(at, role)| (index.shown(*at), role.0.to_owned()))
            .collect(),
    )
}
fn of<'a>(index: &Index, patterns: &'a Patterns, sort: &str) -> Vec<&'a Found> {
    patterns
        .found
        .iter()
        .filter(|found| SORTS[found.sort].id == sort && !index.files.is_empty())
        .collect()
}
fn pairs(parts: &[(&str, &str)]) -> Vec<(String, String)> {
    parts
        .iter()
        .map(|(name, role)| ((*name).to_owned(), (*role).to_owned()))
        .collect()
}
#[test]
fn each_pattern_of_a_shop_is_found_by_what_tells_it() {
    let graph = shop();
    let index = graph.index();
    let patterns = graph.found(&index, false);
    let one = |sort: &str| {
        let found = of(&index, &patterns, sort);
        assert_eq!(found.len(), 1, "{sort}");
        found[0]
    };
    let strategy = one("strategy");
    assert_eq!(
        told(&index, strategy),
        (
            "strategy".to_owned(),
            "PaymentGateway".to_owned(),
            pairs(&[
                ("LoggingGateway", "implementation"),
                ("PaypalGateway", "implementation"),
                ("StripeGateway", "implementation"),
                ("CheckoutService", "user"),
            ])
        )
    );
    assert!(!strategy.named);
    assert_eq!(
        strategy.told,
        [
            "3 implementations stand under PaymentGateway, and 1 other type holds or takes it as that."
        ]
    );
    // What only takes the abstraction where it is built holds it too.
    assert!(
        strategy.links.iter().any(|link| {
            index.shown(link.0) == "CheckoutService" && link.2 == "holds" && !link.3
        })
    );
    let decorator = one("decorator");
    assert_eq!(
        told(&index, decorator),
        (
            "decorator".to_owned(),
            "LoggingGateway".to_owned(),
            pairs(&[
                ("PaymentGateway", "abstraction"),
                ("PaypalGateway", "implementation"),
                ("StripeGateway", "implementation"),
            ])
        )
    );
    assert_eq!(
        decorator
            .links
            .iter()
            .filter(|link| index.shown(link.0) == "LoggingGateway")
            .map(|link| link.2)
            .collect::<Vec<_>>(),
        ["holds", "implements"]
    );
    let factory = one("factory");
    assert_eq!(
        told(&index, factory),
        (
            "factory".to_owned(),
            "GatewayFactory".to_owned(),
            pairs(&[
                ("PaymentGateway", "product"),
                ("PaypalGateway", "made type"),
                ("StripeGateway", "made type"),
            ])
        )
    );
    assert_eq!(
        factory.told,
        ["GatewayFactory.make() returns PaymentGateway and makes PaypalGateway, StripeGateway."]
    );
    // One of the two is made by a call the tool only inferred.
    assert!(factory.guessed() && !factory.named);
    let template = one("template");
    assert_eq!(
        told(&index, template),
        (
            "template".to_owned(),
            "AbstractExporter".to_owned(),
            pairs(&[
                ("CsvExporter", "type that fills the steps"),
                ("HtmlExporter", "type that fills the steps"),
            ])
        )
    );
    assert_eq!(
        template.told,
        [
            "AbstractExporter.export() calls header(), body(), which CsvExporter, HtmlExporter define."
        ]
    );
    let builder = one("builder");
    assert_eq!(
        told(&index, builder),
        (
            "builder".to_owned(),
            "ReportBuilder".to_owned(),
            pairs(&[("Report", "built type")])
        )
    );
    assert!(builder.named);
    assert_eq!(builder.told, ["ReportBuilder.build() gives Report."]);
    let singleton = one("singleton");
    assert_eq!(
        told(&index, singleton),
        (
            "singleton".to_owned(),
            "Logger".to_owned(),
            pairs(&[("LoggingGateway", "user")])
        )
    );
    assert_eq!(
        singleton.told,
        ["Logger.getInstance() hands out the one Logger."]
    );
    // What joins two parts reads as a sentence between their names.
    assert_eq!(
        singleton
            .links
            .iter()
            .map(|link| format!("{} {} {}", index.shown(link.0), link.2, index.shown(link.1)))
            .collect::<Vec<_>>(),
        ["LoggingGateway asks for Logger"]
    );
    let events = one("events");
    assert_eq!(
        told(&index, events),
        (
            "events".to_owned(),
            "OrderPaid".to_owned(),
            pairs(&[
                ("SendReceipt", "listener"),
                ("CheckoutService", "type that raises it")
            ])
        )
    );
    let commands = one("commands");
    assert_eq!(
        told(&index, commands),
        (
            "commands".to_owned(),
            "RefundOrder".to_owned(),
            pairs(&[("RefundOrderHandler", "handler")])
        )
    );
    assert_eq!(
        commands.told,
        ["RefundOrderHandler is named after RefundOrder and takes it."]
    );
    let repository = one("repository");
    assert_eq!(
        told(&index, repository),
        (
            "repository".to_owned(),
            "OrderRepository".to_owned(),
            pairs(&[
                ("Order", "stored type"),
                ("RefundOrderHandler", "user"),
                ("CheckoutService", "user"),
            ])
        )
    );
    assert_eq!(patterns.found.len(), 9);
    // A list goes pattern by pattern, architecture first.
    assert_eq!(
        patterns
            .found
            .iter()
            .map(|found| SORTS[found.sort].id)
            .collect::<Vec<_>>(),
        [
            "repository",
            "events",
            "commands",
            "strategy",
            "decorator",
            "factory",
            "template",
            "builder",
            "singleton"
        ]
    );
}
#[test]
fn links_the_tool_inferred_tell_no_pattern_when_they_are_left_out() {
    let mut graph = shop();
    let index = graph.index();
    // The factory makes one type by a call that was read and one by a call
    // that was inferred: without the second it still makes the first.
    let certain = graph.found(&index, true);
    let factory = of(&index, &certain, "factory")[0];
    assert_eq!(
        told(&index, factory).2,
        pairs(&[
            ("PaymentGateway", "product"),
            ("PaypalGateway", "made type")
        ])
    );
    assert!(!factory.guessed());
    // An abstraction with one implementation read and one inferred is a
    // strategy only by both.
    graph
        .class("Mailer", "app/Mail/Mailer.php", "interface Mailer")
        .method("Mailer", "send")
        .class(
            "SmtpMailer",
            "app/Mail/SmtpMailer.php",
            "class SmtpMailer implements Mailer",
        )
        .class(
            "LogMailer",
            "app/Mail/LogMailer.php",
            "class LogMailer implements Mailer",
        )
        .class("Notifier", "app/Mail/Notifier.php", "class Notifier")
        .link("SmtpMailer", "Mailer", "implements")
        .guessed("LogMailer", "Mailer", "implements")
        .names("Notifier", "Mailer", "field");
    let index = graph.index();
    let anchors = |patterns: &Patterns| {
        of(&index, patterns, "strategy")
            .iter()
            .map(|found| index.shown(found.anchor))
            .collect::<Vec<_>>()
    };
    let every = graph.found(&index, false);
    assert_eq!(anchors(&every), ["PaymentGateway", "Mailer"]);
    assert!(
        every
            .found
            .iter()
            .any(|found| index.shown(found.anchor) == "Mailer" && found.guessed())
    );
    assert_eq!(anchors(&graph.found(&index, true)), ["PaymentGateway"]);
}
#[test]
fn an_abstraction_is_what_the_source_declares_as_one() {
    // Rust implements traits only. `impl From<Money> for Price` is read by
    // the tool as Price implementing Money, which Price also holds.
    let rust = |declared: &str| {
        let mut graph = Graph::default();
        graph
            .class("Money", "src/money.rs", declared)
            .method("Money", "amount")
            .class("Price", "src/price.rs", "pub struct Price")
            .method("Price", "new")
            .class("Cents", "src/cents.rs", "pub struct Cents")
            .class("Till", "src/till.rs", "pub struct Till")
            .link("Price", "Money", "implements")
            .link("Cents", "Money", "implements")
            .names("Price", "Money", "field")
            .names("Till", "Money", "field");
        let index = graph.index();
        let patterns = graph.found(&index, false);
        patterns
            .found
            .iter()
            .map(|found| SORTS[found.sort].id)
            .collect::<Vec<_>>()
    };
    assert_eq!(rust("pub struct Money {"), Vec::<&str>::new());
    assert_eq!(rust("pub trait Money {"), ["strategy", "decorator"]);
    assert_eq!(
        rust("pub(crate) unsafe trait Money: Send {"),
        ["strategy", "decorator"]
    );
    // A word inside another word declares nothing.
    assert_eq!(rust("pub struct Money { // portrait"), Vec::<&str>::new());
    // Elsewhere a class is extended, and what is implemented says so itself.
    let other = |file: &str, declared: &str, relation: &str| {
        let mut graph = Graph::default();
        graph
            .class("Shape", file, declared)
            .method("Shape", "area")
            .class("Circle", "src/Circle.x", "class Circle")
            .class("Square", "src/Square.x", "class Square")
            .class("Canvas", "src/Canvas.x", "class Canvas")
            .link("Circle", "Shape", relation)
            .link("Square", "Shape", relation)
            .names("Canvas", "Shape", "field");
        let index = graph.index();
        graph.found(&index, false).found.len()
    };
    assert_eq!(other("src/Shape.php", "class Shape", "inherits"), 1);
    assert_eq!(
        other("src/Shape.php", "abstract class Shape", "inherits"),
        1
    );
    assert_eq!(
        other("src/Shape.ts", "export interface Shape {", "implements"),
        1
    );
    assert_eq!(other("src/shape.py", "class Shape(ABC):", "inherits"), 1);
    assert_eq!(other("src/Shape.php", "class Shape", "implements"), 0);
    // A source that cannot be read leaves the graph to be believed, where
    // the base has methods of its own.
    let mut unread = Graph::default();
    unread
        .class("Shape", "src/Shape.php", "interface Shape")
        .method("Shape", "area")
        .class("Circle", "src/Circle.php", "class Circle")
        .class("Square", "src/Square.php", "class Square")
        .class("Canvas", "src/Canvas.php", "class Canvas")
        .link("Circle", "Shape", "implements")
        .link("Square", "Shape", "implements")
        .names("Canvas", "Shape", "field");
    let index = unread.index();
    assert_eq!(Patterns::find(&index, false, &|_| None).found.len(), 1);
}
#[test]
fn entities_behind_a_shared_interface_are_no_strategy() {
    // Many entities implement one interface so that one listener can take
    // any of them. They are data that something handles, not ways of doing
    // one thing.
    let built = |under: &[(&str, &str)]| {
        let mut graph = Graph::default();
        graph
            .class("Owned", "src/Entity/Owned.php", "interface Owned")
            .method("Owned", "setOwner")
            .class(
                "AssignOwner",
                "src/EventSubscriber/AssignOwner.php",
                "class AssignOwner",
            )
            .method("AssignOwner", "prePersist")
            .names("AssignOwner.prePersist", "Owned", "parameter_type");
        for (name, file) in under {
            graph
                .class(name, file, &format!("class {name} implements Owned"))
                .link(name, "Owned", "implements");
        }
        let index = graph.index();
        // A type named a repository is found as one beside it, whatever
        // else it is.
        graph
            .found(&index, false)
            .found
            .iter()
            .map(|found| SORTS[found.sort].id)
            .filter(|sort| *sort != "repository")
            .collect::<Vec<_>>()
    };
    let none = Vec::<&str>::new();
    let invoice = ("Invoice", "src/Entity/Invoice.php");
    let order = ("Order", "src/Entity/Order.php");
    // In a folder of the model's own classes.
    assert_eq!(built(&[invoice, order]), none);
    // Named for them wherever they lie, or among them under a name that is
    // a plain noun.
    assert_eq!(
        built(&[
            ("ParcelEntity", "src/Shipping/ParcelEntity.php"),
            ("OAuthClient", "app/Models/OAuthClient.php"),
            ("OrderDto", "src/Api/OrderDto.php"),
        ]),
        none
    );
    // A test double beside them changes nothing.
    assert_eq!(
        built(&[invoice, order, ("FakeOwned", "tests/Support/FakeOwned.php")]),
        none
    );
    // One type under the abstraction that is no thing of the model, and
    // the finding is what it was.
    assert_eq!(
        built(&[invoice, ("SystemOwner", "src/Security/SystemOwner.php")]),
        ["strategy"]
    );
    assert_eq!(
        built(&[
            invoice,
            ("OwnerRepository", "src/Entity/OwnerRepository.php")
        ]),
        ["strategy"]
    );
    // Ways of doing one thing are a strategy where they lie in the domain:
    // a folder of wider meaning than the model's own classes says nothing
    // of what a file is.
    assert_eq!(
        built(&[
            ("FlatPricing", "src/Domain/Pricing/FlatPricing.php"),
            ("TieredPricing", "src/Domain/Pricing/TieredPricing.php"),
        ]),
        ["strategy"]
    );
    // Nothing but test doubles under it is no data either.
    assert_eq!(
        built(&[
            ("FakeOne", "tests/Support/FakeOne.php"),
            ("FakeTwo", "tests/Support/FakeTwo.php"),
        ]),
        ["strategy"]
    );
}
#[test]
fn names_alone_tell_what_links_cannot() {
    let mut graph = Graph::default();
    graph
        .class("Invoice", "app/Invoice.php", "class Invoice")
        .class(
            "InvoiceFactory",
            "app/InvoiceFactory.php",
            "class InvoiceFactory",
        )
        .method("InvoiceFactory", "create")
        .link("InvoiceFactory.create", "Invoice", "calls")
        // A type with fluent setters and getters is no builder.
        .class("Customer", "app/Customer.php", "class Customer")
        .method("Customer", "setName")
        .method("Customer", "getInvoice")
        .names("Customer.getInvoice", "Invoice", "return_type")
        // Nor is a handling method a listener of whatever it takes.
        .class("Importer", "app/Importer.php", "class Importer")
        .method("Importer", "handle")
        .method("Importer", "only")
        .names("Importer.handle", "Invoice", "parameter_type")
        .names("Importer.only", "Customer", "parameter_type")
        // An event by its name, with a method named for the event. A method
        // whose name only begins like one handles nothing.
        .class("Archive", "app/Archive.php", "class Archive")
        .method("Archive", "only")
        .names("Archive.only", "InvoiceSentEvent", "parameter_type")
        // A type that takes its own abstraction in some method wraps none.
        .class("Amount", "app/Amount.php", "interface Amount")
        .method("Amount", "value")
        .class("Price", "app/Price.php", "class Price implements Amount")
        .method("Price", "add")
        .link("Price", "Amount", "implements")
        .names("Price.add", "Amount", "parameter_type")
        // And a method that returns an abstraction makes nothing of it by
        // making something else.
        .class("Registry", "app/Registry.php", "class Registry")
        .method("Registry", "first")
        .names("Registry.first", "Amount", "return_type")
        .link("Registry.first", "Invoice", "calls")
        .class(
            "InvoiceSentEvent",
            "app/InvoiceSentEvent.php",
            "class InvoiceSentEvent",
        )
        .class("Ledger", "app/Ledger.php", "class Ledger")
        .method("Ledger", "onInvoiceSent")
        .names("Ledger.onInvoiceSent", "InvoiceSentEvent", "parameter_type");
    let index = graph.index();
    let patterns = graph.found(&index, false);
    let found = patterns
        .found
        .iter()
        .map(|found| (told(&index, found), found.named))
        .collect::<Vec<_>>();
    assert_eq!(
        found,
        [
            (
                (
                    "events".to_owned(),
                    "InvoiceSentEvent".to_owned(),
                    pairs(&[("Ledger", "listener")])
                ),
                true
            ),
            (
                (
                    "factory".to_owned(),
                    "InvoiceFactory".to_owned(),
                    pairs(&[("Invoice", "made type")])
                ),
                true
            ),
        ]
    );
    assert_eq!(
        patterns.found[1].told,
        ["InvoiceFactory is named a factory and makes Invoice."]
    );
}
#[test]
fn a_picture_carries_the_patterns_of_what_is_open() {
    let index = shop().index();
    let mut view = index.view("demo", "app", None, false, "");
    index.mark_patterns(&mut view, false);
    let marks = view["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| {
            (
                node["path"].as_str().unwrap(),
                node["patterns"].as_u64().unwrap_or(0),
            )
        })
        .collect::<BTreeMap<_, _>>();
    // The sources cannot be read here, so the graph is believed: every
    // finding of the shop is there, and each folder holds those with a part
    // in it.
    assert_eq!(marks["app/Payments"], 4);
    assert_eq!(marks["app/Support"], 3);
    assert_eq!(marks["app/Models"], 1);
    assert_eq!(marks["app/Commands"], 2);
    // With them, and before them, the layers its files are filed in.
    assert_eq!(view["patterns"]["found"], 10);
    let kinds = view["patterns"]["kinds"].as_array().unwrap();
    assert_eq!(
        kinds
            .iter()
            .map(|kind| (
                kind["id"].as_str().unwrap(),
                kind["family"].as_str().unwrap()
            ))
            .collect::<Vec<_>>(),
        [
            ("layers", "architecture"),
            ("repository", "architecture"),
            ("events", "architecture"),
            ("commands", "architecture"),
            ("strategy", "design"),
            ("decorator", "design"),
            ("factory", "design"),
            ("template", "design"),
            ("builder", "design"),
            ("singleton", "design"),
        ]
    );
    assert_eq!(kinds[0]["instances"][0]["id"], "layers@app");
    let strategy = &kinds[4]["instances"][0];
    assert_eq!(
        strategy,
        &json!({
            "id": "strategy@app/Payments/PaymentGateway.php#L3:PaymentGateway",
            "sort": "strategy", "name": "Strategy", "of": "PaymentGateway",
            "file": "app/Payments/PaymentGateway.php", "line": 3,
            "named": false, "guessed": false,
            "parts": "3 implementations · 1 user",
        })
    );
    // An open folder lists what has a part in it, wherever the rest lies.
    let mut view = index.view("demo", "app/Models", None, false, "");
    index.mark_patterns(&mut view, false);
    assert_eq!(view["patterns"]["found"], 1);
    assert_eq!(
        view["patterns"]["kinds"][0]["instances"][0]["of"],
        "OrderRepository"
    );
    // An open file marks the types that take part, and a selected one says
    // in what.
    let gateway = "app/Payments/LoggingGateway.php";
    let key = format!("{gateway}#L3:LoggingGateway");
    let mut view = index
        .symbols("demo", gateway, &[], None, Some(&key), false, "")
        .unwrap();
    index.mark_patterns(&mut view, false);
    let own = view["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["path"] == key.as_str())
        .unwrap();
    assert_eq!(own["patterns"], 3);
    assert_eq!(
        view["focus"]["patterns"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| (row["sort"].as_str().unwrap(), row["of"].as_str().unwrap()))
            .collect::<Vec<_>>(),
        [
            ("strategy", "PaymentGateway"),
            ("decorator", "LoggingGateway"),
            ("singleton", "Logger")
        ]
    );
    assert_eq!(view["patterns"]["found"], 3);
    // Without the links the tool inferred the factory makes one type less.
    let mut view = index.view("demo", "app/Payments", None, true, "");
    index.mark_patterns(&mut view, true);
    // A file is marked by the findings with a part in it.
    let marks = view["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| {
            (
                node["name"].as_str().unwrap(),
                node["patterns"].as_u64().unwrap_or(0),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        marks,
        BTreeMap::from([
            ("GatewayFactory.php", 1),
            ("LoggingGateway.php", 3),
            ("PaymentGateway.php", 3),
            ("PaypalGateway.php", 3),
            ("StripeGateway.php", 2),
        ])
    );
    let factory = view["patterns"]["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .find(|kind| kind["id"] == "factory")
        .unwrap();
    assert_eq!(factory["instances"][0]["parts"], "1 product · 1 made type");
    assert_eq!(factory["instances"][0]["guessed"], false);
}
#[test]
fn one_finding_is_drawn_with_what_each_part_is_in_it() {
    let index = shop().index();
    let id = "decorator@app/Payments/LoggingGateway.php#L3:LoggingGateway";
    let view = index
        .pattern("demo", id, &[], &|_| None, None, false)
        .unwrap();
    assert_eq!(view["mode"], "pattern");
    assert_eq!(view["scope"], "app/Payments/LoggingGateway.php");
    assert_eq!(
        view["crumbs"].as_array().unwrap().last().unwrap()["name"],
        "Decorator: LoggingGateway"
    );
    assert_eq!(view["pattern"]["name"], "Decorator");
    assert_eq!(view["pattern"]["family"], "design");
    assert_eq!(view["pattern"]["of"]["name"], "LoggingGateway");
    assert_eq!(view["pattern"]["role"], "decorator");
    assert_eq!(
        view["pattern"]["parts"],
        json!([
            {"one": "abstraction", "many": "abstractions", "found": 1, "drawn": 1},
            {"one": "implementation", "many": "implementations", "found": 2, "drawn": 2},
        ])
    );
    let nodes = view["nodes"].as_array().unwrap();
    assert_eq!(
        nodes
            .iter()
            .map(|node| {
                (
                    node["name"].as_str().unwrap(),
                    node["role"].as_str().unwrap(),
                    node["start"] == true,
                )
            })
            .collect::<Vec<_>>(),
        [
            ("LoggingGateway", "decorator", true),
            ("PaymentGateway", "abstraction", false),
            ("PaypalGateway", "implementation", false),
            ("StripeGateway", "implementation", false),
        ]
    );
    assert!(
        nodes
            .iter()
            .all(|node| node["symbol"] == true && node["kind"] == "type")
    );
    // Two parts are joined once, by all that joins them.
    assert_eq!(
        view["edges"],
        json!([
            {
                "from": id.split_once('@').unwrap().1,
                "to": "app/Payments/PaymentGateway.php#L3:PaymentGateway",
                "links": 1, "guessed": 0, "tie": "holds and implements",
            },
            {
                "from": "app/Payments/PaypalGateway.php#L3:PaypalGateway",
                "to": "app/Payments/PaymentGateway.php#L3:PaymentGateway",
                "links": 1, "guessed": 0, "tie": "implements",
            },
            {
                "from": "app/Payments/StripeGateway.php#L3:StripeGateway",
                "to": "app/Payments/PaymentGateway.php#L3:PaymentGateway",
                "links": 1, "guessed": 0, "tie": "implements",
            },
        ])
    );
    // A selected part is listed with what joins it to the others.
    let abstraction = "app/Payments/PaymentGateway.php#L3:PaymentGateway";
    let view = index
        .pattern("demo", id, &[], &|_| None, Some(abstraction), false)
        .unwrap();
    assert_eq!(view["focus"]["role"], "abstraction");
    assert_eq!(view["focus"]["uses"], json!([]));
    assert_eq!(
        view["focus"]["used_by"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| (
                row["name"].as_str().unwrap(),
                row["relation"].as_str().unwrap(),
                row["role"].as_str().unwrap()
            ))
            .collect::<Vec<_>>(),
        [
            ("LoggingGateway", "holds and implements", "decorator"),
            ("PaypalGateway", "implements", "implementation"),
            ("StripeGateway", "implements", "implementation"),
        ]
    );
    // A finding the graph does not have is no picture.
    for gone in [
        "decorator@app/Payments/StripeGateway.php#L3:StripeGateway",
        "strategy@app/Payments/LoggingGateway.php#L3:LoggingGateway",
        "decorator@app/Payments/LoggingGateway.php",
        "decorator@gone.php#L1:Gone",
        "layers@gone",
        "layers@app/Models",
        "layer@app",
        "decorator",
        "",
    ] {
        assert!(
            index
                .pattern("demo", gone, &[], &|_| None, None, false)
                .is_none(),
            "{gone}"
        );
    }
}
#[test]
fn a_large_finding_draws_a_share_of_each_role() {
    let mut graph = Graph::default();
    graph
        .class("Rule", "src/Rule.php", "interface Rule")
        .method("Rule", "passes");
    for at in 0..70 {
        let name = format!("Rule{at:02}");
        graph
            .class(&name, &format!("src/rules/{name}.php"), "class")
            .link(&name, "Rule", "implements");
    }
    for at in 0..5 {
        let name = format!("Check{at}");
        graph
            .class(&name, &format!("src/checks/{name}.php"), "class")
            .names(&name, "Rule", "field");
    }
    let index = graph.index();
    let id = "strategy@src/Rule.php#L3:Rule";
    let view = index
        .pattern("demo", id, &[], &|_| None, None, false)
        .unwrap();
    assert_eq!(view["nodes"].as_array().unwrap().len(), DRAWN);
    // The few users are all drawn, and the implementations take the rest.
    assert_eq!(
        view["pattern"]["parts"],
        json!([
            {"one": "implementation", "many": "implementations", "found": 70, "drawn": 34},
            {"one": "user", "many": "users", "found": 5, "drawn": 5},
        ])
    );
    assert_eq!(view["edges"].as_array().unwrap().len(), DRAWN - 1);
    // Two roles that are both too large halve the picture between them.
    for at in 5..30 {
        let name = format!("Check{at}");
        graph
            .class(&name, &format!("src/checks/{name}.php"), "class")
            .names(&name, "Rule", "field");
    }
    let index = graph.index();
    let view = index
        .pattern("demo", id, &[], &|_| None, None, false)
        .unwrap();
    assert_eq!(view["nodes"].as_array().unwrap().len(), DRAWN);
    assert_eq!(
        view["pattern"]["parts"],
        json!([
            {"one": "implementation", "many": "implementations", "found": 70, "drawn": 20},
            {"one": "user", "many": "users", "found": 30, "drawn": 19},
        ])
    );
    assert_eq!(
        view["frames"],
        json!([
            {"path": "role:0", "name": "Abstraction", "holder": null, "places": 1, "drawn": 1, "fixed": true},
            {"path": "role:1", "name": "Implementations", "holder": null, "places": 70, "drawn": 20, "fixed": true},
            {"path": "role:2", "name": "Users", "holder": null, "places": 30, "drawn": 19, "fixed": true},
        ])
    );
    assert_eq!(view["nodes"][0]["holder"], "role:0");
    assert_eq!(view["nodes"][1]["holder"], "role:1");
    assert_eq!(view["nodes"][DRAWN - 1]["holder"], "role:2");
    // A list names a pattern's largest findings and counts the rest.
    let mut many = Graph::default();
    for at in 0..LISTED + 4 {
        let name = format!("Thing{at:02}Repository");
        many.class(&name, &format!("src/{name}.php"), "class");
    }
    let index = many.index();
    let mut view = index.view("demo", "", None, false, "");
    index.mark_patterns(&mut view, false);
    let kind = &view["patterns"]["kinds"][0];
    assert_eq!(
        (kind["found"].as_u64(), kind["more"].as_u64()),
        (Some(34), Some(4))
    );
    assert_eq!(kind["instances"].as_array().unwrap().len(), LISTED);
}
#[test]
fn patterns_are_those_of_the_graph_that_is_shown() {
    // The second implementation is a test double: without tests there is
    // one implementation, and no strategy.
    let mut graph = Graph::default();
    graph
        .class("Clock", "src/Clock.php", "interface Clock")
        .method("Clock", "now")
        .class("SystemClock", "src/SystemClock.php", "class SystemClock")
        .class("FrozenClock", "tests/FrozenClock.php", "class FrozenClock")
        .class("Scheduler", "src/Scheduler.php", "class Scheduler")
        .link("SystemClock", "Clock", "implements")
        .link("FrozenClock", "Clock", "implements")
        .names("Scheduler", "Clock", "field");
    let index = graph.index();
    let found = |index: &Index| {
        let mut view = index.view("demo", "", None, false, "");
        index.mark_patterns(&mut view, false);
        view["patterns"]["found"].as_u64().unwrap()
    };
    assert_eq!(found(&index), 1);
    assert_eq!(found(&index.without_tests()), 0);
    // The contexts the patterns read are kept from the graph, the others
    // are not.
    assert_eq!(how("field"), Some(FIELD));
    assert_eq!(how("parameter_type"), Some(PARAMETER));
    assert_eq!(how("return_type"), Some(RESULT));
    assert_eq!(how("generic_arg"), Some(GENERIC));
    assert_eq!(how("constructor"), Some(MADE));
    assert_eq!(how("call"), None);
    assert_eq!(index.ties.len(), 1);
}
