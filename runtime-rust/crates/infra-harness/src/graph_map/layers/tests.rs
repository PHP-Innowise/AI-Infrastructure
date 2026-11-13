//! Tests of the layers a folder is filed in.
use super::*;

/// A graph of types, one in each file, and the links between them.
#[derive(Default)]
struct Graph {
    nodes: Vec<Value>,
    links: Vec<Value>,
}
impl Graph {
    /// A type named after its file.
    fn file(&mut self, path: &str) -> &mut Self {
        let name = path.rsplit('/').next().unwrap().split('.').next().unwrap();
        self.nodes.push(json!({
            "id": path, "label": name, "source_file": path, "source_location": "L3",
            "_callable_class": true,
        }));
        self
    }
    /// The type of one file needs the type of another: a relation, and for
    /// a reference where in the code it stands.
    fn link(&mut self, from: &str, to: &str, relation: &str, context: Option<&str>) -> &mut Self {
        self.links.push(json!({
            "source": from, "target": to, "relation": relation, "context": context,
            "confidence": "EXTRACTED",
        }));
        self
    }
    fn guessed(&mut self, from: &str, to: &str) -> &mut Self {
        self.links.push(json!({
            "source": from, "target": to, "relation": "calls", "confidence": "INFERRED",
        }));
        self
    }
    fn index(&self) -> Index {
        let graph = json!({"nodes": self.nodes, "links": self.links});
        Index::read(graph.to_string().as_bytes(), Path::new("/work/shop")).unwrap()
    }
}
const CONTROLLER: &str = "app/Http/Controllers/CheckoutController.php";
const MIDDLEWARE: &str = "app/Http/Middleware/Authenticate.php";
const SERVICE: &str = "app/Services/CheckoutService.php";
const LISTENER: &str = "app/Listeners/SendReceipt.php";
const REPOSITORY: &str = "app/Repositories/OrderRepository.php";
const GATEWAY: &str = "app/Payments/StripeGateway.php";
const ORDER: &str = "app/Models/Order.php";
const PAID: &str = "app/Events/OrderPaid.php";
const REPORT: &str = "app/Support/Report.php";
const TEST: &str = "tests/Services/CheckoutServiceTest.php";
/// A shop in four layers, with two links against their order.
fn shop() -> Graph {
    let mut graph = Graph::default();
    for path in [
        CONTROLLER,
        MIDDLEWARE,
        SERVICE,
        LISTENER,
        REPOSITORY,
        GATEWAY,
        ORDER,
        PAID,
        REPORT,
        TEST,
        "docs/services/overview.md",
    ] {
        graph.file(path);
    }
    graph
        .link(CONTROLLER, SERVICE, "calls", None)
        .link(CONTROLLER, SERVICE, "references", Some("field"))
        .link(MIDDLEWARE, SERVICE, "calls", None)
        .link(SERVICE, REPOSITORY, "references", Some("field"))
        .link(SERVICE, GATEWAY, "calls", None)
        .link(SERVICE, ORDER, "references", Some("parameter_type"))
        .link(SERVICE, PAID, "calls", None)
        .link(LISTENER, PAID, "references", Some("parameter_type"))
        .link(REPOSITORY, ORDER, "references", Some("return_type"))
        .link(GATEWAY, ORDER, "inherits", None)
        // Against the order: the model holds a service, and data access
        // calls an entry point by a link the tool only inferred.
        .link(ORDER, SERVICE, "references", Some("field"))
        .link(ORDER, SERVICE, "calls", None)
        .guessed(REPOSITORY, CONTROLLER)
        // Knowing of a file is not needing it: an import, a name in an
        // attribute.
        .link(ORDER, REPOSITORY, "imports", Some("import"))
        .link(PAID, CONTROLLER, "references", Some("decorator"))
        .link(PAID, CONTROLLER, "references", None)
        // Within one layer, with what no layer holds, and from a test.
        .link(SERVICE, LISTENER, "calls", None)
        .link(SERVICE, REPORT, "calls", None)
        .link(REPORT, CONTROLLER, "calls", None)
        .link(TEST, SERVICE, "calls", None)
        .link("docs/services/overview.md", CONTROLLER, "calls", None);
    graph
}
#[test]
fn a_file_is_filed_by_its_name_or_by_the_folder_it_lies_in() {
    let named = |path: &str| layer(path).map(|layer| LAYERS[layer].name);
    for (path, expected) in [
        // By the ending of the name, as a word of it.
        (
            "app/Http/Controllers/CheckoutController.php",
            Some("Entry points"),
        ),
        ("src/Checkout/CheckoutController.php", Some("Entry points")),
        ("src/Checkout/CheckoutService.php", Some("Services")),
        ("src/Checkout/RefundOrderHandler.php", Some("Services")),
        ("src/Checkout/OrderRepository.php", Some("Data access")),
        (
            "src/Checkout/OrderRepositoryInterface.php",
            Some("Data access"),
        ),
        ("src/Checkout/StripeGateway.php", Some("Data access")),
        ("src/Checkout/OrderEntity.php", Some("Model")),
        ("src/orders/order_service.py", Some("Services")),
        ("src/orders/orders.controller.ts", Some("Entry points")),
        ("src/orders/orders-repository.js", Some("Data access")),
        ("src/Customer.php", None),
        ("src/Remodel.php", None),
        ("src/Datamapper.php", None),
        ("src/Заказ.php", None),
        ("src/ЗаказService.php", Some("Services")),
        ("src/Ünïcödé-service.php", Some("Services")),
        // Letters of more bytes than one, where an ending would begin inside
        // one, and a capital that grows when it is made small.
        ("src/éé.php", None),
        ("src/ééé.php", None),
        ("src/İService.php", Some("Services")),
        ("src/İ.php", None),
        // By its whole name, as Django files are named.
        ("shop/models.py", Some("Model")),
        ("shop/views.py", Some("Entry points")),
        ("shop/services.py", Some("Services")),
        ("src/orders/repository.rs", Some("Data access")),
        // An ending that is the whole name, where no folder is named so.
        ("src/billing/client.py", Some("Data access")),
        ("src/Gateway.ts", Some("Data access")),
        ("src/jobs/job.rb", Some("Services")),
        // A name that files a folder files no file so named.
        ("src/Application.java", None),
        ("config/application.rb", None),
        ("src/domain.rs", None),
        ("src/application/checkout.rs", Some("Services")),
        ("routes/console.php", Some("Entry points")),
        // By the nearest folder named for a layer.
        ("app/Models/Order.php", Some("Model")),
        ("src/Entity/Order.php", Some("Model")),
        ("app/Events/OrderPaid.php", Some("Model")),
        ("app/Listeners/SendReceipt.php", Some("Services")),
        ("src/EventSubscriber/Locale.php", Some("Services")),
        ("app/Console/Commands/Prune.php", Some("Entry points")),
        ("src/Domain/Service/Pricing.php", Some("Services")),
        ("src/Domain/Order/Line.php", Some("Model")),
        // The name says more than the folder.
        ("app/Services/OrderRepository.php", Some("Data access")),
        // Not where it is a plain noun in a folder of the model's own
        // classes: an entity is called after what it is.
        ("src/Entity/OAuthClient.php", Some("Model")),
        ("src/Entity/NewsletterSubscriber.php", Some("Model")),
        ("src/Entity/AccountManager.php", Some("Model")),
        ("app/Models/Job.php", Some("Model")),
        ("app/Models/Billing/HostingService.php", Some("Model")),
        ("src/Dto/WebhookEndpoint.php", Some("Model")),
        ("shop/models/gateway.py", Some("Model")),
        ("shop/models/service.py", Some("Model")),
        // What only does something is no thing of the model, there too.
        ("src/Entity/UserRepository.php", Some("Data access")),
        ("src/Entity/OrderListener.php", Some("Services")),
        ("app/Models/OrderController.php", Some("Entry points")),
        // A folder nearer than the model's files as before.
        ("src/Entity/Listener/OrderSubscriber.php", Some("Services")),
        ("src/Model/Service/PriceService.php", Some("Services")),
        // A folder that holds more than the model's classes does not make
        // a noun of a name.
        ("src/Domain/Billing/InvoiceService.php", Some("Services")),
        ("app/Events/OrderSubscriber.php", Some("Services")),
        ("src/Security/Sso/OidcClient.php", Some("Data access")),
        ("src/Service/Client.php", Some("Data access")),
        // After a dot `client` says where a file runs, not what it is.
        ("plugins/analytics.client.ts", None),
        ("src/api/http.client.ts", None),
        ("src/api/http-client.ts", Some("Data access")),
        ("src/api/HttpClient.ts", Some("Data access")),
        ("src/api/http_client.py", Some("Data access")),
        ("src/clients/analytics.client.ts", Some("Data access")),
        // Folders that hold more than what their name says file nothing.
        ("app/Http/Requests/StoreOrder.php", None),
        ("src/GraphQL/Type/OrderType.php", None),
        ("src/GraphQL/Scalars/Date.php", None),
        ("src/GraphQL/Schema.php", None),
        ("src/Command/ImportOrders.php", None),
        ("app/Actions/CreateOrder.php", None),
        // Inside `graphql` a folder of operations holds entry points, as a
        // folder of resolvers does anywhere.
        (
            "src/GraphQL/Operation/Mutation/CreateOrder.php",
            Some("Entry points"),
        ),
        (
            "src/GraphQL/Operation/AbstractOperation.php",
            Some("Entry points"),
        ),
        ("app/GraphQL/Queries/Orders.php", Some("Entry points")),
        (
            "app/GraphQL/Mutations/Billing/PayOrder.php",
            Some("Entry points"),
        ),
        (
            "src/graphql/subscriptions/orderPaid.ts",
            Some("Entry points"),
        ),
        ("src/Api/Graphql/Query/Orders.php", Some("Entry points")),
        // Anywhere else it holds messages as often, and files nothing.
        ("src/Query/GetOrders.php", None),
        ("app/Mutations/CreateOrder.php", None),
        ("src/Application/Query/GetOrders.php", Some("Services")),
        ("src/Query/GraphQL/Orders.php", None),
        // A nearer folder and a name say more there as everywhere.
        (
            "src/GraphQL/Mutation/Handler/CreateOrder.php",
            Some("Services"),
        ),
        (
            "src/GraphQL/Mutation/OrderRepository.php",
            Some("Data access"),
        ),
        ("src/GraphQL/Query/Model/OAuthClient.php", Some("Model")),
        // A test is filed in no layer, nor is what is no code.
        ("tests/Services/CheckoutServiceTest.php", None),
        ("src/Services/__tests__/checkout.service.ts", None),
        ("docs/services/overview.md", None),
        ("config/services.yaml", None),
        // Nor is a template that wears the ending of a language.
        ("resources/views/orders/show.blade.php", None),
        ("shop/views/orders.py", Some("Entry points")),
        ("services", None),
    ] {
        assert_eq!(named(path), expected, "{path}");
    }
}
#[test]
fn a_file_is_one_of_the_models_own_classes_by_its_name_or_its_folder() {
    // The order the code names two layers by.
    assert_eq!((LAYERS[ENTRY].id, LAYERS[MODEL].id), ("entry", "model"));
    for (path, expected) in [
        ("src/Entity/Order.php", true),
        ("src/Entity/OAuthClient.php", true),
        ("app/Models/Billing/Invoice.php", true),
        ("src/Billing/OrderEntity.php", true),
        ("src/Api/OrderDto.php", true),
        ("shop/models.py", true),
        ("shop/billing/entity.ts", true),
        // What a folder of wider meaning files in the model may be anything.
        ("src/Domain/Order/Line.php", false),
        ("app/Events/OrderPaid.php", false),
        ("shop/events.py", false),
        // What is filed elsewhere, or nowhere, is none of them.
        ("src/Entity/UserRepository.php", false),
        ("src/Entity/Listener/OrderListener.php", false),
        ("src/Service/Pricing.php", false),
        ("src/Support/Money.php", false),
        ("tests/Entity/OrderTest.php", false),
        ("docs/entity/order.md", false),
    ] {
        assert_eq!(own(path), expected, "{path}");
    }
}
#[test]
fn the_layers_of_a_folder_say_what_needs_what_and_what_runs_against_their_order() {
    let index = shop().index();
    let view = index
        .pattern("demo", "layers@app", &[], &|_| None, None, false)
        .unwrap();
    assert_eq!(view["mode"], "pattern");
    assert_eq!(view["scope"], "app");
    assert_eq!(
        view["crumbs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|crumb| crumb["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["demo", "app", "Layers"]
    );
    let pattern = &view["pattern"];
    assert_eq!(pattern["id"], "layers@app");
    assert_eq!(pattern["sort"], "layers");
    assert_eq!(pattern["family"], "architecture");
    assert_eq!(
        pattern["of"],
        json!({"path": "app", "name": "app", "what": "folder"})
    );
    assert_eq!(
        pattern["layers"],
        json!([
            {"id": "entry", "name": "Entry points", "files": 2},
            {"id": "services", "name": "Services", "files": 2},
            {"id": "data", "name": "Data access", "files": 2},
            {"id": "model", "name": "Model", "files": 2},
        ])
    );
    assert_eq!(pattern["apart"], 1);
    assert_eq!(
        pattern["told"],
        json!(["1 file of code here is filed in no layer."])
    );
    // A layer is one box, and a link between two of them all that the
    // files of the one need of the files of the other.
    assert_eq!(
        view["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| {
                (
                    node["path"].as_str().unwrap(),
                    node["files"].as_u64().unwrap(),
                    node["out"].as_u64().unwrap(),
                    node["in"].as_u64().unwrap(),
                )
            })
            .collect::<Vec<_>>(),
        [
            ("layer:entry", 2, 3, 1),
            ("layer:services", 2, 5, 5),
            ("layer:data", 2, 3, 2),
            ("layer:model", 2, 2, 5),
        ]
    );
    assert!(
        view["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|node| { node["folder"] == true && node["layer"] == true })
    );
    assert_eq!(
        view["edges"],
        json!([
            {"from": "layer:entry", "to": "layer:services", "links": 3, "guessed": 0, "against": false},
            {"from": "layer:services", "to": "layer:data", "links": 2, "guessed": 0, "against": false},
            {"from": "layer:services", "to": "layer:model", "links": 3, "guessed": 0, "against": false},
            {"from": "layer:data", "to": "layer:entry", "links": 1, "guessed": 1, "against": true},
            {"from": "layer:data", "to": "layer:model", "links": 2, "guessed": 0, "against": false},
            {"from": "layer:model", "to": "layer:services", "links": 2, "guessed": 0, "against": true},
        ])
    );
    // What runs against the order is listed pair by pair, the most linked
    // first: an import and a name in an attribute are no need.
    assert_eq!(
        pattern["against"],
        json!({
            "links": 3, "guessed": 1, "pairs": 2, "more": 0,
            "rows": [
                {
                    "from": {"path": ORDER, "name": "Order.php", "layer": "Model"},
                    "to": {"path": SERVICE, "name": "CheckoutService.php", "layer": "Services"},
                    "links": 2, "guessed": 0,
                },
                {
                    "from": {"path": REPOSITORY, "name": "OrderRepository.php", "layer": "Data access"},
                    "to": {"path": CONTROLLER, "name": "CheckoutController.php", "layer": "Entry points"},
                    "links": 1, "guessed": 1,
                },
            ],
        })
    );
    assert_eq!(pattern["guessed"], true);
    assert_eq!(view["focus"], Value::Null);
    // Without the links the tool inferred the call back to the entry point
    // is gone, and with it that layer's use by another.
    let certain = index
        .pattern("demo", "layers@app", &[], &|_| None, None, true)
        .unwrap();
    assert_eq!(certain["pattern"]["against"]["links"], 2);
    assert_eq!(certain["pattern"]["against"]["pairs"], 1);
    assert_eq!(certain["pattern"]["guessed"], false);
    assert_eq!(certain["edges"].as_array().unwrap().len(), 5);
    assert_eq!(certain["nodes"][0]["in"], 0);
}
#[test]
fn a_selected_layer_lists_its_files_and_what_it_has_with_the_others() {
    let index = shop().index();
    let view = index
        .pattern(
            "demo",
            "layers@app",
            &[],
            &|_| None,
            Some("layer:model"),
            false,
        )
        .unwrap();
    let focus = &view["focus"];
    assert_eq!(focus["path"], "layer:model");
    assert_eq!(focus["name"], "Model");
    assert_eq!(focus["files"], 2);
    assert_eq!(
        focus["signs"],
        "a name that ends in model, entity, dto, valueobject, or a folder named models, model, entities, entity, domain, dto, dtos, valueobjects, events, event"
    );
    // The entry points also say what files an operation of a GraphQL API.
    let entry = index
        .pattern(
            "demo",
            "layers@app",
            &[],
            &|_| None,
            Some("layer:entry"),
            false,
        )
        .unwrap();
    assert_eq!(
        entry["focus"]["signs"],
        "a name that ends in controller, middleware, endpoint, or a folder named controller, controllers, console, middleware, resolver, resolvers, views, routes, or inside graphql a folder named mutation, mutations, operation, operations, query, queries, subscription, subscriptions"
    );
    // Its files, by how much each has with the other layers.
    assert_eq!(
        focus["members"],
        json!([
            {"path": ORDER, "name": "Order.php", "folder": false, "inside": false, "links": 5, "guessed": 0, "changed": false},
            {"path": PAID, "name": "OrderPaid.php", "folder": false, "inside": false, "links": 2, "guessed": 0, "changed": false},
        ])
    );
    assert_eq!(focus["members_more"], 0);
    let beside = |rows: &Value| {
        rows.as_array()
            .unwrap()
            .iter()
            .map(|row| {
                (
                    row["name"].as_str().unwrap().to_owned(),
                    row["links"].as_u64().unwrap(),
                    row["against"] == true,
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(beside(&focus["uses"]), [("Services".to_owned(), 2, true)]);
    assert_eq!(
        beside(&focus["used_by"]),
        [
            ("Services".to_owned(), 3, false),
            ("Data access".to_owned(), 2, false)
        ]
    );
    assert_eq!(focus["against"]["rows"].as_array().unwrap().len(), 1);
    assert_eq!(focus["against"]["rows"][0]["from"]["path"], ORDER);
    assert_eq!(focus["against"]["more"], 0);
    // A layer that needs nothing before it has nothing against the order.
    let entry = index
        .pattern(
            "demo",
            "layers@app",
            &[],
            &|_| None,
            Some("layer:entry"),
            false,
        )
        .unwrap();
    assert_eq!(entry["focus"]["against"]["rows"], json!([]));
    assert_eq!(
        beside(&entry["focus"]["used_by"]),
        [("Data access".to_owned(), 1, true)]
    );
    // What is no layer of the picture selects nothing.
    for other in ["layer:none", SERVICE, "app", ""] {
        let view = index
            .pattern("demo", "layers@app", &[], &|_| None, Some(other), false)
            .unwrap();
        assert_eq!(view["focus"], Value::Null, "{other}");
    }
}
#[test]
fn a_selected_link_between_two_layers_lists_the_pairs_of_files_it_is_made_of() {
    let index = shop().index();
    let link = |between: &str, certain: bool| {
        index
            .pattern("demo", "layers@app", &[], &|_| None, Some(between), certain)
            .unwrap()["focus"]
            .clone()
    };
    let pairs = |focus: &Value| {
        focus["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                (
                    row["from"]["path"].as_str().unwrap().to_owned(),
                    row["to"]["path"].as_str().unwrap().to_owned(),
                    row["links"].as_u64().unwrap(),
                    row["guessed"].as_u64().unwrap(),
                )
            })
            .collect::<Vec<_>>()
    };
    let pair = |from: &str, to: &str, links: u64, guessed: u64| {
        (from.to_owned(), to.to_owned(), links, guessed)
    };
    // With the order: every pair of a file of the one layer and a file of
    // the other that it needs, of as many links in the order of the paths.
    let focus = link("layer:services>layer:model", false);
    assert_eq!(focus["path"], "layer:services>layer:model");
    assert_eq!(focus["link"], true);
    assert_eq!(
        focus["from"],
        json!({"path": "layer:services", "name": "Services"})
    );
    assert_eq!(focus["to"], json!({"path": "layer:model", "name": "Model"}));
    assert_eq!(
        (
            &focus["links"],
            &focus["guessed"],
            &focus["against"],
            &focus["pairs"],
            &focus["more"]
        ),
        (&json!(3), &json!(0), &json!(false), &json!(3), &json!(0))
    );
    assert_eq!(
        pairs(&focus),
        [
            pair(LISTENER, PAID, 1, 0),
            pair(SERVICE, PAID, 1, 0),
            pair(SERVICE, ORDER, 1, 0),
        ]
    );
    assert_eq!(focus["rows"][0]["from"]["name"], "SendReceipt.php");
    assert_eq!(focus["rows"][0]["from"]["layer"], "Services");
    assert_eq!(focus["rows"][0]["to"]["layer"], "Model");
    // The same two layers the other way round are another link, and it
    // runs against the order.
    let focus = link("layer:model>layer:services", false);
    assert_eq!(focus["against"], true);
    assert_eq!((&focus["links"], &focus["pairs"]), (&json!(2), &json!(1)));
    assert_eq!(pairs(&focus), [pair(ORDER, SERVICE, 2, 0)]);
    let focus = link("layer:entry>layer:services", false);
    assert_eq!(
        pairs(&focus),
        [
            pair(CONTROLLER, SERVICE, 2, 0),
            pair(MIDDLEWARE, SERVICE, 1, 0)
        ]
    );
    // A link the tool only inferred is one until only what was read from
    // the code is asked for.
    let focus = link("layer:data>layer:entry", false);
    assert_eq!(
        (&focus["links"], &focus["guessed"], &focus["against"]),
        (&json!(1), &json!(1), &json!(true))
    );
    assert_eq!(pairs(&focus), [pair(REPOSITORY, CONTROLLER, 1, 1)]);
    assert_eq!(link("layer:data>layer:entry", true), Value::Null);
    // What is no link of the picture selects nothing: two layers without a
    // link that way, a layer and itself, a layer that is not there.
    for none in [
        "layer:entry>layer:model",
        "layer:services>layer:entry",
        "layer:model>layer:model",
        "layer:entry>layer:none",
        "layer:entry>",
        ">layer:entry",
        ">",
        "layer:entry>layer:services>layer:model",
        "entry>services",
    ] {
        assert_eq!(link(none, false), Value::Null, "{none}");
    }
}
#[test]
fn layers_are_those_of_the_folder_that_is_open() {
    let index = shop().index();
    let layers = |scope: &str| {
        index.pattern(
            "demo",
            &format!("layers@{scope}"),
            &[],
            &|_| None,
            None,
            false,
        )
    };
    // The whole project: the test and the document are filed nowhere, and
    // only code is counted as left apart.
    let whole = layers("").unwrap();
    assert_eq!(whole["scope"], "");
    assert_eq!(
        whole["pattern"]["of"],
        json!({"path": "", "name": "demo", "what": "project"})
    );
    assert_eq!(whole["pattern"]["layers"].as_array().unwrap().len(), 4);
    assert_eq!(whole["pattern"]["apart"], 1);
    assert_eq!(
        whole["crumbs"].as_array().unwrap().last().unwrap()["name"],
        "Layers"
    );
    // A folder of one layer has no layers, nor has one whose layers no
    // link joins, a file, or a folder the graph does not know.
    for none in [
        "app/Http",
        "app/Services",
        "app/Support",
        SERVICE,
        "gone",
        "app/",
    ] {
        assert!(layers(none).is_none(), "{none}");
    }
    let mut apart = Graph::default();
    for path in [CONTROLLER, MIDDLEWARE, ORDER, PAID, REPORT] {
        apart.file(path);
    }
    apart.link(CONTROLLER, REPORT, "calls", None);
    apart.link(ORDER, REPORT, "calls", None);
    apart.link(CONTROLLER, MIDDLEWARE, "calls", None);
    apart.link(ORDER, PAID, "calls", None);
    assert!(
        apart
            .index()
            .pattern("demo", "layers@app", &[], &|_| None, None, false)
            .is_none()
    );
    // Where patterns are asked for, the layers of the open folder come
    // first among them.
    let kinds = |view: &Value| {
        view["patterns"]["kinds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|kind| kind["id"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    let mut view = index.view("demo", "app", None, false, "");
    index.mark_patterns(&mut view, false);
    assert_eq!(kinds(&view), ["layers", "repository"]);
    assert_eq!(view["patterns"]["found"], 2);
    assert_eq!(
        view["patterns"]["kinds"][0],
        json!({
            "id": "layers", "name": "Layers", "family": "architecture", "tells": TELLS,
            "found": 1, "more": 0,
            "instances": [{
                "id": "layers@app", "sort": "layers", "name": "Layers", "of": "app",
                "file": "", "line": 0, "named": true, "guessed": false,
                "parts": "4 layers · 8 files · 3 links against their order",
            }],
        })
    );
    let mut certain = index.view("demo", "", None, true, "");
    index.mark_patterns(&mut certain, true);
    let row = &certain["patterns"]["kinds"][0]["instances"][0];
    assert_eq!(row["id"], "layers@");
    assert_eq!(row["of"], "the whole project");
    assert_eq!(
        row["parts"],
        "4 layers · 8 files · 2 links against their order"
    );
    // A folder of one layer has other findings, and no layers.
    let mut inside = index.view("demo", "app/Services", None, false, "");
    index.mark_patterns(&mut inside, false);
    assert_eq!(kinds(&inside), ["repository"]);
    // A project in order says so.
    let mut clean = Graph::default();
    clean.file(CONTROLLER).file(SERVICE).file(ORDER).file(PAID);
    clean.link(CONTROLLER, SERVICE, "calls", None);
    clean.link(SERVICE, ORDER, "calls", None);
    clean.link(SERVICE, PAID, "calls", None);
    let index = clean.index();
    let mut view = index.view("demo", "", None, false, "");
    index.mark_patterns(&mut view, false);
    assert_eq!(
        view["patterns"]["kinds"][0]["instances"][0]["parts"],
        "3 layers · 4 files · nothing against their order"
    );
    let view = index
        .pattern("demo", "layers@", &[], &|_| None, None, false)
        .unwrap();
    assert_eq!(view["pattern"]["against"]["rows"], json!([]));
    assert_eq!(view["pattern"]["told"], json!([]));
    assert!(
        view["edges"]
            .as_array()
            .unwrap()
            .iter()
            .all(|edge| edge["against"] == false)
    );
}
#[test]
fn the_layers_of_a_folder_count_what_lies_in_it_and_nothing_else() {
    // Two parts of a project, each in layers of its own, that also need
    // each other: against the order only from one part to the other.
    let mut graph = Graph::default();
    let file = |part: &str, name: &str| format!("src/{part}/{name}.php");
    for part in ["Billing", "Shop"] {
        for name in [
            "Controller/PayController",
            "Service/PayService",
            "Entity/Invoice",
            "Entity/Line",
            "Util/Money",
        ] {
            graph.file(&file(part, name));
        }
        graph
            .link(
                &file(part, "Controller/PayController"),
                &file(part, "Service/PayService"),
                "calls",
                None,
            )
            .link(
                &file(part, "Service/PayService"),
                &file(part, "Entity/Invoice"),
                "calls",
                None,
            );
    }
    graph
        .link(
            &file("Billing", "Entity/Invoice"),
            &file("Shop", "Service/PayService"),
            "calls",
            None,
        )
        .link(
            &file("Shop", "Controller/PayController"),
            &file("Billing", "Entity/Line"),
            "calls",
            None,
        );
    let index = graph.index();
    let read = |scope: &str| {
        let view = index
            .pattern(
                "demo",
                &format!("layers@{scope}"),
                &[],
                &|_| None,
                None,
                false,
            )
            .unwrap();
        (
            view["pattern"]["layers"]
                .as_array()
                .unwrap()
                .iter()
                .map(|layer| layer["files"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            view["pattern"]["apart"].as_u64().unwrap(),
            view["edges"]
                .as_array()
                .unwrap()
                .iter()
                .map(|edge| edge["links"].as_u64().unwrap())
                .sum::<u64>(),
            view["pattern"]["against"]["links"].as_u64().unwrap(),
        )
    };
    assert_eq!(read("src"), (vec![2, 2, 4], 2, 6, 1));
    assert_eq!(read(""), (vec![2, 2, 4], 2, 6, 1));
    assert_eq!(read("src/Billing"), (vec![1, 1, 2], 1, 2, 0));
    assert_eq!(read("src/Shop"), (vec![1, 1, 2], 1, 2, 0));
    // A selected layer lists the files of the open folder, with the links
    // they have in it.
    let members = |scope: &str| {
        let view = index
            .pattern(
                "demo",
                &format!("layers@{scope}"),
                &[],
                &|_| None,
                Some("layer:model"),
                false,
            )
            .unwrap();
        view["focus"]["members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|file| {
                (
                    file["path"].as_str().unwrap().to_owned(),
                    file["links"].as_u64().unwrap(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        members("src/Billing"),
        [
            (file("Billing", "Entity/Invoice"), 1),
            (file("Billing", "Entity/Line"), 0)
        ]
    );
    assert_eq!(
        members("src"),
        [
            (file("Billing", "Entity/Invoice"), 2),
            (file("Billing", "Entity/Line"), 1),
            (file("Shop", "Entity/Invoice"), 1),
            (file("Shop", "Entity/Line"), 0),
        ]
    );
    // So does a selected link list the pairs of the open folder.
    let made = |scope: &str, link: &str| {
        let view = index
            .pattern(
                "demo",
                &format!("layers@{scope}"),
                &[],
                &|_| None,
                Some(link),
                false,
            )
            .unwrap();
        view["focus"]["rows"].as_array().map(|rows| {
            rows.iter()
                .map(|row| {
                    (
                        row["from"]["path"].as_str().unwrap().to_owned(),
                        row["to"]["path"].as_str().unwrap().to_owned(),
                    )
                })
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(
        made("src/Billing", "layer:entry>layer:services"),
        Some(vec![(
            file("Billing", "Controller/PayController"),
            file("Billing", "Service/PayService")
        )])
    );
    assert_eq!(
        made("src", "layer:entry>layer:services").map(|pairs| pairs.len()),
        Some(2)
    );
    // The link against the order leaves each part, and is one of the whole.
    assert_eq!(made("src/Billing", "layer:model>layer:services"), None);
    assert_eq!(
        made("src", "layer:model>layer:services"),
        Some(vec![(
            file("Billing", "Entity/Invoice"),
            file("Shop", "Service/PayService")
        )])
    );
}
#[test]
fn an_entity_called_after_a_plain_noun_is_no_layer_of_its_own() {
    // `OAuthClient` among the entities is an entity. Filed as data access by
    // its name it made a folder of entities one of two layers, and every
    // entity that holds it a link against their order.
    let mut graph = Graph::default();
    let entity = |name: &str| format!("src/Entity/{name}.php");
    for name in ["OAuthClient", "OAuthToken", "Invoice", "Order", "User"] {
        graph.file(&entity(name));
    }
    graph.file("src/Controller/TokenController.php");
    graph
        .link(
            &entity("OAuthToken"),
            &entity("OAuthClient"),
            "references",
            Some("field"),
        )
        .link(
            &entity("OAuthClient"),
            &entity("User"),
            "references",
            Some("field"),
        )
        .link(
            &entity("Order"),
            &entity("Invoice"),
            "references",
            Some("field"),
        )
        .link(
            "src/Controller/TokenController.php",
            &entity("OAuthClient"),
            "calls",
            None,
        );
    let index = graph.index();
    let layers = |scope: &str| {
        index.pattern(
            "demo",
            &format!("layers@{scope}"),
            &[],
            &|_| None,
            None,
            false,
        )
    };
    assert!(layers("src/Entity").is_none());
    let view = layers("src").unwrap();
    assert_eq!(
        view["pattern"]["layers"],
        json!([
            {"id": "entry", "name": "Entry points", "files": 1},
            {"id": "model", "name": "Model", "files": 5},
        ])
    );
    assert_eq!(view["pattern"]["against"]["links"], 0);
    assert_eq!(
        view["edges"],
        json!([{"from": "layer:entry", "to": "layer:model", "links": 1, "guessed": 0, "against": false}])
    );
}
#[test]
fn a_few_files_named_like_layers_are_no_layered_design() {
    // Layers hold no fewer than four files, and no less than a tenth of the
    // code of the folder: tests and what is no code do not count.
    let built = |filed: usize, other: usize| {
        let mut graph = Graph::default();
        let paths = [CONTROLLER, SERVICE, ORDER, PAID, LISTENER];
        for path in &paths[..filed] {
            graph.file(path);
        }
        graph.link(CONTROLLER, SERVICE, "calls", None);
        graph.link(SERVICE, ORDER, "calls", None);
        for at in 0..other {
            graph.file(&format!("app/Support/Tool{at:02}.php"));
        }
        for at in 0..50 {
            graph.file(&format!("tests/Support/Tool{at:02}Test.php"));
            graph.file(&format!("docs/tool{at:02}.md"));
            graph.file(&format!("resources/views/tool{at:02}.blade.php"));
        }
        let index = graph.index();
        let found = index
            .pattern("demo", "layers@", &[], &|_| None, None, false)
            .map(|view| view["pattern"]["apart"].as_u64().unwrap());
        let mut view = index.view("demo", "", None, false, "");
        index.mark_patterns(&mut view, false);
        // The list and the picture agree on whether there are layers.
        assert_eq!(
            view["patterns"]["kinds"][0]["id"] == "layers",
            found.is_some(),
            "{filed} {other}"
        );
        found
    };
    assert_eq!(built(3, 0), None);
    assert_eq!(built(4, 0), Some(0));
    assert_eq!(built(4, 36), Some(36));
    assert_eq!(built(4, 37), None);
    assert_eq!(built(5, 45), Some(45));
    assert_eq!(built(5, 46), None);
}
#[test]
fn many_files_and_many_pairs_are_listed_the_most_linked_first() {
    let mut graph = Graph::default();
    let service = |at: usize| format!("app/Services/Step{at:02}Service.php");
    let model = |at: usize| format!("app/Models/Thing{at:02}.php");
    for at in 0..LISTED + 5 {
        graph.file(&service(at)).file(&model(at));
    }
    for at in 0..LISTED + 5 {
        // Every model needs a service, against the order.
        graph.link(&model(at), &service(at), "calls", None);
        // The later a model the more services need it.
        for other in 0..=at.min(3) {
            graph.link(&service(other), &model(at), "references", Some("field"));
        }
    }
    let index = graph.index();
    let view = index
        .pattern(
            "demo",
            "layers@app",
            &[],
            &|_| None,
            Some("layer:model"),
            false,
        )
        .unwrap();
    let against = &view["pattern"]["against"];
    assert_eq!(against["pairs"], LISTED + 5);
    assert_eq!(against["rows"].as_array().unwrap().len(), LISTED);
    assert_eq!(against["more"], 5);
    // Of as many links, in the order of the paths.
    assert_eq!(against["rows"][0]["from"]["name"], "Thing00.php");
    assert_eq!(against["rows"][1]["from"]["name"], "Thing01.php");
    let members = view["focus"]["members"].as_array().unwrap();
    assert_eq!(members.len(), LISTED);
    assert_eq!(view["focus"]["members_more"], 5);
    assert_eq!(members[0]["name"], "Thing03.php");
    assert_eq!(members[0]["links"], 5);
    assert_eq!(members[LISTED - 1]["links"], 5);
    assert_eq!(view["focus"]["against"]["more"], 5);
    // A link of more pairs than fit lists the first of them and says how
    // many it left out.
    let link = |between: &str| {
        index
            .pattern("demo", "layers@app", &[], &|_| None, Some(between), false)
            .unwrap()["focus"]
            .clone()
    };
    let back = link("layer:model>layer:services");
    assert_eq!(back["pairs"], LISTED + 5);
    assert_eq!(back["rows"].as_array().unwrap().len(), LISTED);
    assert_eq!(back["more"], 5);
    assert_eq!(back["links"], LISTED + 5);
    // Four services need each model from the fourth on, and fewer the three
    // before it.
    let forth = link("layer:services>layer:model");
    assert_eq!(forth["pairs"], 1 + 2 + 3 + 4 * (LISTED + 2));
    assert_eq!(forth["rows"].as_array().unwrap().len(), LISTED);
    assert_eq!(forth["more"], 1 + 2 + 3 + 4 * (LISTED + 2) - LISTED);
    assert_eq!(forth["rows"][0]["from"]["name"], "Step00Service.php");
    assert_eq!(forth["rows"][0]["to"]["name"], "Thing00.php");
    assert_eq!(forth["rows"][1]["to"]["name"], "Thing01.php");
}
