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
        // A folder of wide meaning files what no narrower one does, and the
        // outermost of them speaks: one inside it is as often named for
        // what the code is about.
        (
            "src/Application/Command/Domain/RegisterDomain.php",
            Some("Services"),
        ),
        ("src/Domain/Application/Line.php", Some("Model")),
        ("src/Infrastructure/Bus/CommandBus.php", Some("Data access")),
        (
            "src/Infrastructure/Controller/Orders.php",
            Some("Entry points"),
        ),
        ("src/Service/Domain/Pricing.php", Some("Services")),
        ("src/Domain/ValueObject/Money.php", Some("Model")),
        ("src/Domain/ValueObject/Client.php", Some("Model")),
        // What passes between the layers is of none, wherever it lies.
        ("src/Service/Bank/Request/SubmitRequest.php", None),
        ("src/Service/Bank/Exception/TokenException.php", None),
        ("src/Service/Bank/Exception/Retry.php", None),
        ("src/Service/OrderResponse.php", None),
        ("src/Api/order_response.py", None),
        // Not among the model's own classes: there it is a thing of the
        // model, as any other noun.
        ("src/Entity/OrderResponse.php", Some("Model")),
        ("src/Entity/AccessRequest.php", Some("Model")),
        ("app/Models/Request.php", Some("Model")),
        ("app/Http/Controllers/Requests.php", Some("Entry points")),
        ("src/Service/Exceptional.php", Some("Services")),
        // An ending that names a thing as often gives way to a folder that
        // says another layer, and to no other.
        (
            "src/GraphQL/Operation/Query/FindPhoneModel.php",
            Some("Entry points"),
        ),
        ("src/Service/LegalEntity.php", Some("Services")),
        ("src/GraphQL/Type/PhoneModel.php", Some("Model")),
        ("src/Billing/PhoneModel.php", Some("Model")),
        ("src/Domain/Billing/PhoneModel.php", Some("Model")),
        ("src/Controller/Dto/OrderDto.php", Some("Model")),
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
fn a_file_lies_in_the_ring_of_the_outermost_folder_named_for_one() {
    let named = |path: &str| ring(path).map(|ring| RINGS[ring].name);
    for (path, expected) in [
        ("src/Domain/Entity/Order.php", Some("Domain")),
        (
            "src/Domain/Repository/OrderRepositoryInterface.php",
            Some("Domain"),
        ),
        ("src/Domain/Exception/OrderNotFound.php", Some("Domain")),
        (
            "src/Application/Command/CreateOrder.php",
            Some("Application"),
        ),
        ("src/Application/DTO/OrderDto.php", Some("Application")),
        (
            "src/Infrastructure/Controller/OrderController.php",
            Some("Adapters"),
        ),
        (
            "src/Infrastructure/Repository/OrderRepository.php",
            Some("Adapters"),
        ),
        ("src/UI/Http/OrderController.php", Some("Adapters")),
        ("src/Presentation/Cli/ImportOrders.php", Some("Adapters")),
        ("app/infra/db/orders.ts", Some("Adapters")),
        // The outermost speaks: a folder inside is named for what the code
        // is about.
        (
            "src/Application/Command/Domain/RegisterDomain.php",
            Some("Application"),
        ),
        ("src/Domain/Infrastructure/Line.php", Some("Domain")),
        ("src/Billing/Domain/Invoice.php", Some("Domain")),
        (
            "src/Billing/Infrastructure/Application/Mailer.php",
            Some("Adapters"),
        ),
        // Outside the rings, a test, and what is no code.
        ("src/Kernel.php", None),
        ("src/Command/ImportOrders.php", None),
        ("migrations/Version1.php", None),
        ("domain.php", None),
        ("tests/Domain/OrderTest.php", None),
        ("src/Domain/readme.md", None),
    ] {
        assert_eq!(named(path), expected, "{path}");
    }
}
/// A shop that keeps its domain at the heart: a domain with the interfaces
/// of its repositories, an application of commands and their handlers, and
/// adapters around them.
fn rings() -> Graph {
    let mut graph = Graph::default();
    for path in [
        "src/Domain/Entity/Order.php",
        "src/Domain/Entity/Line.php",
        "src/Domain/Repository/OrderRepositoryInterface.php",
        "src/Domain/Factory/OrderFactory.php",
        "src/Domain/Exception/OrderNotFound.php",
        "src/Application/Command/Order/PlaceOrder.php",
        "src/Application/Command/Domain/RenameDomain.php",
        "src/Application/CommandHandler/PlaceOrderHandler.php",
        "src/Application/Bus/CommandBusInterface.php",
        "src/Infrastructure/Controller/OrderController.php",
        "src/Infrastructure/Repository/OrderRepository.php",
        "src/Infrastructure/Bus/CommandBus.php",
        "src/Kernel.php",
        "tests/Domain/OrderTest.php",
    ] {
        graph.file(path);
    }
    graph
        // An adapter implements what the domain and the application ask
        // for, and calls into them.
        .link(
            "src/Infrastructure/Repository/OrderRepository.php",
            "src/Domain/Repository/OrderRepositoryInterface.php",
            "implements",
            None,
        )
        .link(
            "src/Infrastructure/Repository/OrderRepository.php",
            "src/Domain/Entity/Order.php",
            "references",
            Some("return_type"),
        )
        .link(
            "src/Infrastructure/Bus/CommandBus.php",
            "src/Application/Bus/CommandBusInterface.php",
            "implements",
            None,
        )
        .link(
            "src/Infrastructure/Controller/OrderController.php",
            "src/Application/Command/Order/PlaceOrder.php",
            "calls",
            None,
        )
        .link(
            "src/Infrastructure/Controller/OrderController.php",
            "src/Infrastructure/Bus/CommandBus.php",
            "calls",
            None,
        )
        // The application works with the domain, through the interface of
        // its repository.
        .link(
            "src/Application/CommandHandler/PlaceOrderHandler.php",
            "src/Domain/Repository/OrderRepositoryInterface.php",
            "references",
            Some("field"),
        )
        .link(
            "src/Application/CommandHandler/PlaceOrderHandler.php",
            "src/Domain/Factory/OrderFactory.php",
            "calls",
            None,
        )
        .link(
            "src/Application/CommandHandler/PlaceOrderHandler.php",
            "src/Application/Command/Order/PlaceOrder.php",
            "references",
            Some("parameter_type"),
        )
        // Inside the domain a factory asks the repository for what it needs.
        .link(
            "src/Domain/Factory/OrderFactory.php",
            "src/Domain/Repository/OrderRepositoryInterface.php",
            "references",
            Some("field"),
        )
        .link(
            "src/Domain/Entity/Order.php",
            "src/Domain/Entity/Line.php",
            "references",
            Some("field"),
        )
        // And once the domain reaches out of itself.
        .link(
            "src/Domain/Entity/Order.php",
            "src/Infrastructure/Bus/CommandBus.php",
            "calls",
            None,
        )
        .guessed(
            "src/Application/Command/Domain/RenameDomain.php",
            "src/Infrastructure/Controller/OrderController.php",
        );
    graph
}
#[test]
fn a_folder_that_keeps_its_code_in_rings_is_read_by_them() {
    let index = rings().index();
    let view = index
        .pattern("demo", "layers@src", &[], &|_| None, None, false)
        .unwrap();
    let pattern = &view["pattern"];
    assert_eq!(pattern["rings"], true);
    assert_eq!(pattern["tells"], TELLS[RINGED]);
    assert_eq!(
        pattern["layers"],
        json!([
            {"id": "adapters", "name": "Adapters", "files": 3},
            {"id": "application", "name": "Application", "files": 4},
            {"id": "domain", "name": "Domain", "files": 5},
        ])
    );
    // What lies in no ring is left apart: the kernel, and not the test.
    assert_eq!(pattern["apart"], 1);
    assert_eq!(
        view["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node["path"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["layer:adapters", "layer:application", "layer:domain"]
    );
    // An adapter may use the application and the domain, the application
    // the domain. What a ring has inside itself is no link between rings:
    // the factory and the repository of the domain, the controller and the
    // bus among the adapters.
    assert_eq!(
        view["edges"],
        json!([
            {"from": "layer:adapters", "to": "layer:application", "links": 2, "guessed": 0, "against": false},
            {"from": "layer:adapters", "to": "layer:domain", "links": 2, "guessed": 0, "against": false},
            {"from": "layer:application", "to": "layer:adapters", "links": 1, "guessed": 1, "against": true},
            {"from": "layer:application", "to": "layer:domain", "links": 2, "guessed": 0, "against": false},
            {"from": "layer:domain", "to": "layer:adapters", "links": 1, "guessed": 0, "against": true},
        ])
    );
    let against = &pattern["against"];
    assert_eq!(
        (&against["links"], &against["pairs"]),
        (&json!(2), &json!(2))
    );
    assert_eq!(
        against["rows"][0],
        json!({
            "from": {
                "path": "src/Application/Command/Domain/RenameDomain.php",
                "name": "RenameDomain.php", "layer": "Application",
            },
            "to": {
                "path": "src/Infrastructure/Controller/OrderController.php",
                "name": "OrderController.php", "layer": "Adapters",
            },
            "links": 1, "guessed": 1,
        })
    );
    assert_eq!(against["rows"][1]["from"]["layer"], "Domain");
    // Read by the four layers the same shop would have its adapters run
    // against the order wherever they do what they are for.
    let layered = Way::read(&index, false, &layer);
    let back = layered
        .cross
        .iter()
        .filter(|(from, to, _)| layered.of[*from as usize] > layered.of[*to as usize])
        .count();
    assert!(back > 2, "{back}");
    // The list says so in the words of the rings, and a selected ring says
    // what files a file in it.
    let mut listed = index.view("demo", "src", None, false, "");
    index.mark_patterns(&mut listed, false);
    let kind = &listed["patterns"]["kinds"][0];
    assert_eq!(kind["tells"], TELLS[RINGED]);
    assert_eq!(
        kind["instances"][0]["parts"],
        "3 layers · 12 files · 2 links against their order"
    );
    let ring = index
        .pattern(
            "demo",
            "layers@src",
            &[],
            &|_| None,
            Some("layer:domain"),
            false,
        )
        .unwrap();
    assert_eq!(ring["focus"]["name"], "Domain");
    assert_eq!(ring["focus"]["signs"], "the outermost folder named domain");
    assert_eq!(ring["focus"]["members"].as_array().unwrap().len(), 5);
    let outer = index
        .pattern(
            "demo",
            "layers@src",
            &[],
            &|_| None,
            Some("layer:adapters"),
            false,
        )
        .unwrap();
    assert_eq!(
        outer["focus"]["signs"],
        "the outermost folder named infrastructure, infra, presentation, ui, adapters, adapter"
    );
    let link = index
        .pattern(
            "demo",
            "layers@src",
            &[],
            &|_| None,
            Some("layer:domain>layer:adapters"),
            false,
        )
        .unwrap();
    assert_eq!(link["focus"]["against"], true);
    assert_eq!(link["focus"]["pairs"], 1);
    // The whole project is read the same way, and without what the tool
    // only inferred one link back is gone.
    let whole = index
        .pattern("demo", "layers@", &[], &|_| None, None, true)
        .unwrap();
    assert_eq!(whole["pattern"]["rings"], true);
    assert_eq!(whole["pattern"]["against"]["links"], 1);
}
#[test]
fn inside_one_ring_and_where_rings_hold_little_a_folder_is_read_by_the_four_layers() {
    let index = rings().index();
    let way = |scope: &str| {
        index
            .pattern(
                "demo",
                &format!("layers@{scope}"),
                &[],
                &|_| None,
                None,
                false,
            )
            .map(|view| view["pattern"]["rings"] == true)
    };
    // Inside the domain there is one ring. By the four layers its factory
    // is of the model and the interface of its repository of data access,
    // but four files in layers are wanted, and here are five.
    let domain = index
        .pattern("demo", "layers@src/Domain", &[], &|_| None, None, false)
        .unwrap();
    assert_eq!(domain["pattern"]["rings"], false);
    assert_eq!(
        domain["pattern"]["layers"],
        json!([
            {"id": "data", "name": "Data access", "files": 1},
            {"id": "model", "name": "Model", "files": 3},
        ])
    );
    // The exception passes between the layers and is of none.
    assert_eq!(domain["pattern"]["apart"], 1);
    assert_eq!(way("src/Infrastructure"), None);
    // Rings that hold less than half of the code do not speak for it: the
    // folder is read by the four layers, as any other.
    let mut graph = rings();
    let mut classic = Vec::new();
    for at in 0..8 {
        classic.push(format!("src/Controller/Page{at}Controller.php"));
        classic.push(format!("src/Entity/Page{at}.php"));
    }
    for path in &classic {
        graph.file(path);
    }
    for pair in classic.chunks(2) {
        graph.link(&pair[0], &pair[1], "calls", None);
    }
    let index = graph.index();
    let view = index
        .pattern("demo", "layers@src", &[], &|_| None, None, false)
        .unwrap();
    assert_eq!(view["pattern"]["rings"], false);
    assert_eq!(view["pattern"]["tells"], TELLS[LAYERED]);
    assert_eq!(view["pattern"]["layers"][0]["name"], "Entry points");
    // As many files in the rings as outside them, and they speak.
    let mut graph = rings();
    for path in classic.iter().take(11) {
        graph.file(path);
    }
    for pair in classic.chunks(2).take(5) {
        graph.link(&pair[0], &pair[1], "calls", None);
    }
    let index = graph.index();
    let view = index
        .pattern("demo", "layers@src", &[], &|_| None, None, false)
        .unwrap();
    assert_eq!(view["pattern"]["rings"], true);
    assert_eq!(view["pattern"]["apart"], 12);
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
        "a name that ends in model, entity, dto, valueobject, or a folder named models, model, entities, entity, domain, dto, dtos, valueobject, valueobjects, events, event"
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
            "id": "layers", "name": "Layers", "family": "architecture", "tells": TELLS[LAYERED],
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
/// The shop as it was before a change, and as the change left it: the
/// model's order reaches for a service where it did not, the repository no
/// longer calls the controller, and the gateway was moved.
fn changed() -> (Graph, Graph) {
    let build = |gateway: &str, links: &[(&str, &str)]| {
        let mut graph = Graph::default();
        for path in [
            CONTROLLER, MIDDLEWARE, SERVICE, LISTENER, REPOSITORY, ORDER, PAID,
        ] {
            graph.file(path);
        }
        graph.file(gateway);
        graph
            .link(CONTROLLER, SERVICE, "calls", None)
            .link(SERVICE, REPOSITORY, "calls", None)
            .link(SERVICE, ORDER, "calls", None)
            .link(REPOSITORY, ORDER, "calls", None)
            // Against the order at both ends of the change: the gateway
            // asks a service.
            .link(gateway, SERVICE, "calls", None);
        for (from, to) in links {
            graph.link(from, to, "calls", None);
        }
        graph
    };
    (
        build(GATEWAY, &[(REPOSITORY, CONTROLLER), (PAID, LISTENER)]),
        build(
            "app/Billing/StripeGateway.php",
            &[
                (ORDER, SERVICE),
                (ORDER, SERVICE),
                (PAID, LISTENER),
                (PAID, LISTENER),
            ],
        ),
    )
}
#[test]
fn a_change_says_which_pairs_it_set_against_the_order_and_which_it_took_away() {
    let (before, after) = changed();
    let (base, index) = (before.index(), after.index());
    let moved = HashMap::from([("app/Billing/StripeGateway.php", GATEWAY)]);
    let compared = |focus: Option<&str>, renamed: &HashMap<&str, &str>| {
        let mut view = index
            .pattern("demo", "layers@app", &[], &|_| None, focus, false)
            .unwrap();
        index.compare_layers(&base, &mut view, renamed, false);
        view
    };
    let ends = |rows: &Value| {
        rows.as_array()
            .unwrap()
            .iter()
            .map(|row| {
                (
                    row["from"]["path"].as_str().unwrap().to_owned(),
                    row["to"]["path"].as_str().unwrap().to_owned(),
                    row["links"].as_u64().unwrap(),
                )
            })
            .collect::<Vec<_>>()
    };
    let view = compared(None, &moved);
    let change = &view["pattern"]["change"];
    assert_eq!(change["state"], "ready");
    // Added: the order and the service. The event and its listener ran
    // against the order before, with one link where there are two now, and
    // a pair is told by its files; so did the gateway, under another path.
    assert_eq!(
        (
            &change["added"]["pairs"],
            &change["added"]["links"],
            &change["added"]["more"]
        ),
        (&json!(1), &json!(2), &json!(0))
    );
    assert_eq!(
        ends(&change["added"]["rows"]),
        [(ORDER.to_owned(), SERVICE.to_owned(), 2)]
    );
    assert_eq!(change["added"]["rows"][0]["from"]["layer"], "Model");
    assert_eq!(
        change["added"]["rows"][0]["to"]["name"],
        "CheckoutService.php"
    );
    // Taken away: the repository and the controller, as the base has them.
    assert_eq!(
        (&change["removed"]["pairs"], &change["removed"]["links"]),
        (&json!(1), &json!(1))
    );
    assert_eq!(
        ends(&change["removed"]["rows"]),
        [(REPOSITORY.to_owned(), CONTROLLER.to_owned(), 1)]
    );
    assert_eq!(change["removed"]["rows"][0]["from"]["layer"], "Data access");
    // The rows of the picture that are among the added say so, and no other.
    let marks = |rows: &Value| {
        rows.as_array()
            .unwrap()
            .iter()
            .map(|row| {
                (
                    row["from"]["name"].as_str().unwrap().to_owned(),
                    row["added"] == true,
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        marks(&view["pattern"]["against"]["rows"]),
        [
            ("OrderPaid.php".to_owned(), false),
            ("Order.php".to_owned(), true),
            ("StripeGateway.php".to_owned(), false),
        ]
    );
    // A link between two layers says how many of its links the change added.
    let added = |view: &Value| {
        view["edges"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|edge| !edge["added"].is_null())
            .map(|edge| {
                (
                    edge["from"].as_str().unwrap().to_owned(),
                    edge["to"].as_str().unwrap().to_owned(),
                    edge["added"].as_u64().unwrap(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        added(&view),
        [("layer:model".to_owned(), "layer:services".to_owned(), 2)]
    );
    // So do the rows of a selected layer and of a selected link.
    let layer = compared(Some("layer:model"), &moved);
    assert_eq!(
        marks(&layer["focus"]["against"]["rows"]),
        [
            ("OrderPaid.php".to_owned(), false),
            ("Order.php".to_owned(), true)
        ]
    );
    let link = compared(Some("layer:model>layer:services"), &moved);
    assert_eq!(
        marks(&link["focus"]["rows"]),
        [
            ("OrderPaid.php".to_owned(), false),
            ("Order.php".to_owned(), true)
        ]
    );
    // Without knowing that the gateway was moved, it is a pair the change
    // added and another it took away.
    let unknown = compared(None, &HashMap::new());
    let change = &unknown["pattern"]["change"];
    assert_eq!(
        (&change["added"]["pairs"], &change["removed"]["pairs"]),
        (&json!(2), &json!(2))
    );
    assert_eq!(
        ends(&change["added"]["rows"]),
        [
            (ORDER.to_owned(), SERVICE.to_owned(), 2),
            (
                "app/Billing/StripeGateway.php".to_owned(),
                SERVICE.to_owned(),
                1
            ),
        ]
    );
    // The same graph at both ends, and the change did nothing to the order.
    let mut same = index
        .pattern("demo", "layers@app", &[], &|_| None, None, false)
        .unwrap();
    index.compare_layers(&index, &mut same, &HashMap::new(), false);
    let change = &same["pattern"]["change"];
    assert_eq!(
        (&change["added"]["pairs"], &change["removed"]["pairs"]),
        (&json!(0), &json!(0))
    );
    assert_eq!(change["added"]["rows"], json!([]));
    assert!(added(&same).is_empty());
    assert!(
        same["pattern"]["against"]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["added"].is_null())
    );
}
#[test]
fn a_change_is_measured_against_whatever_its_base_was() {
    let (_, after) = changed();
    let index = after.index();
    let compared = |base: &Index, scope: &str, certain: bool| {
        let mut view = index
            .pattern(
                "demo",
                &format!("layers@{scope}"),
                &[],
                &|_| None,
                None,
                certain,
            )
            .unwrap();
        index.compare_layers(base, &mut view, &HashMap::new(), certain);
        view["pattern"]["change"].clone()
    };
    // A base without layers: every pair against the order is the change's.
    let mut flat = Graph::default();
    flat.file("app/Support/Report.php")
        .file("app/Support/Money.php");
    let change = compared(&flat.index(), "app", false);
    assert_eq!(
        (&change["added"]["pairs"], &change["removed"]["pairs"]),
        (&json!(3), &json!(0))
    );
    // The base is read the way the folder is read now. A shop in rings
    // gains pages enough to be read by the four layers: of the pairs that
    // run against either order the change itself made none and took none
    // away, and the picture says so, though by its rings the base had a
    // handler against the order that the four layers have not.
    let (handler, repository) = (
        "src/Application/CommandHandler/PlaceOrderHandler.php",
        "src/Infrastructure/Repository/OrderRepository.php",
    );
    let mut ringed = rings();
    ringed.link(handler, repository, "calls", None);
    let mut grown = rings();
    grown.link(handler, repository, "calls", None);
    for at in 0..8 {
        grown
            .file(&format!("src/Controller/Page{at}Controller.php"))
            .file(&format!("src/Entity/Page{at}.php"))
            .link(
                &format!("src/Controller/Page{at}Controller.php"),
                &format!("src/Entity/Page{at}.php"),
                "calls",
                None,
            );
    }
    let (ringed, layered) = (ringed.index(), grown.index());
    let read = |index: &Index, base: &Index| {
        let mut view = index
            .pattern("demo", "layers@src", &[], &|_| None, None, false)
            .unwrap();
        index.compare_layers(base, &mut view, &HashMap::new(), false);
        (
            view["pattern"]["rings"] == true,
            view["pattern"]["against"]["pairs"].as_u64().unwrap(),
            view["pattern"]["change"]["added"]["pairs"]
                .as_u64()
                .unwrap(),
            view["pattern"]["change"]["removed"]["pairs"]
                .as_u64()
                .unwrap(),
        )
    };
    assert_eq!(read(&layered, &ringed), (false, 4, 0, 0));
    // A handler that takes a repository of the adapters runs against the
    // rings and not against the four layers: read by its rings, the shop
    // has it from the change.
    assert_eq!(read(&ringed, &rings().index()), (true, 3, 1, 0));
    assert_eq!(read(&rings().index(), &ringed), (true, 2, 0, 1));
    // The other way round the pages are gone and the shop is read by its
    // rings again: three pairs against their order, none of them new.
    assert_eq!(read(&ringed, &layered), (true, 3, 0, 0));
    // One more link out of the domain, and it is the one the change added.
    let mut worse = rings();
    worse.link(handler, repository, "calls", None).link(
        "src/Domain/Entity/Line.php",
        "src/Application/Bus/CommandBusInterface.php",
        "calls",
        None,
    );
    assert_eq!(read(&worse.index(), &layered), (true, 4, 1, 0));
    // What the tool only inferred is left out at both ends when asked.
    let mut guessing = Graph::default();
    for path in [
        CONTROLLER, MIDDLEWARE, SERVICE, LISTENER, REPOSITORY, ORDER, PAID, GATEWAY,
    ] {
        guessing.file(path);
    }
    guessing
        .link(CONTROLLER, SERVICE, "calls", None)
        .link(SERVICE, ORDER, "calls", None)
        .guessed(ORDER, SERVICE);
    let base = guessing.index();
    let change = compared(&base, "app", false);
    assert_eq!(change["added"]["pairs"], 2);
    let change = compared(&base, "app", true);
    assert_eq!(change["added"]["pairs"], 3);
    // Of what a change added, what the tool only inferred is counted so.
    let mut view = base
        .pattern("demo", "layers@app", &[], &|_| None, None, false)
        .unwrap();
    base.compare_layers(&flat.index(), &mut view, &HashMap::new(), false);
    let added = &view["pattern"]["change"]["added"];
    assert_eq!(
        (&added["pairs"], &added["links"], &added["guessed"]),
        (&json!(1), &json!(1), &json!(1))
    );
    assert_eq!(added["rows"][0]["guessed"], 1);
    // More pairs than fit are listed the most linked first and counted.
    let mut many = Graph::default();
    for at in 0..LISTED + 5 {
        let (service, model) = (
            format!("app/Services/Step{at:02}Service.php"),
            format!("app/Models/Thing{at:02}.php"),
        );
        many.file(&service).file(&model);
        many.link(&model, &service, "calls", None);
        if at == 7 {
            many.link(&model, &service, "references", Some("field"));
        }
    }
    let index = many.index();
    let mut view = index
        .pattern("demo", "layers@app", &[], &|_| None, None, false)
        .unwrap();
    index.compare_layers(&flat.index(), &mut view, &HashMap::new(), false);
    let added = &view["pattern"]["change"]["added"];
    assert_eq!(added["pairs"], LISTED + 5);
    assert_eq!(added["links"], LISTED + 6);
    assert_eq!(added["rows"].as_array().unwrap().len(), LISTED);
    assert_eq!(added["more"], 5);
    assert_eq!(added["rows"][0]["from"]["name"], "Thing07.php");
}
#[test]
fn what_is_moved_with_a_folder_or_a_file_is_neither_added_nor_taken_away() {
    let files = [
        "Http/Controllers/CheckoutController.php",
        "Services/CheckoutService.php",
        "Repositories/OrderRepository.php",
        "Models/Order.php",
    ];
    // A shop under a folder, and beside it an invoice with its service in
    // a folder of their own. `back` says which of the two models asks its
    // service, against the order: the order, the invoice.
    let shop = |root: &str, beside: &str, back: (bool, bool)| {
        let at = |path: &str| format!("{root}/{path}");
        let mut graph = Graph::default();
        for path in files {
            graph.file(&at(path));
        }
        for (from, to) in [(0, 1), (1, 2), (2, 3)] {
            graph.link(&at(files[from]), &at(files[to]), "calls", None);
        }
        if back.0 {
            graph.link(&at(files[3]), &at(files[1]), "calls", None);
        }
        let (invoice, invoices) = (
            format!("{beside}/Models/Invoice.php"),
            format!("{beside}/Services/InvoiceService.php"),
        );
        graph.file(&invoice).file(&invoices);
        graph.link(&invoices, &invoice, "calls", None);
        if back.1 {
            graph.link(&invoice, &invoices, "calls", None);
        }
        graph
    };
    // Against the order in a folder now, added there by the change, and
    // taken away there by it.
    let did = |base: &Graph, head: &Graph, scope: &str, renamed: &[(&str, &str)]| {
        let (base, index) = (base.index(), head.index());
        let mut view = index
            .pattern(
                "demo",
                &format!("layers@{scope}"),
                &[],
                &|_| None,
                None,
                false,
            )
            .unwrap();
        index.compare_layers(&base, &mut view, &renamed.iter().copied().collect(), false);
        let change = &view["pattern"]["change"];
        (
            view["pattern"]["against"]["pairs"].as_u64().unwrap(),
            change["added"]["pairs"].as_u64().unwrap(),
            change["removed"]["pairs"].as_u64().unwrap(),
        )
    };
    let both = (true, true);
    // The folder of the picture is renamed: what it had against the order
    // it has still, and the change did none of it. Where the files are not
    // known to be the same, the folder is all new, and the pair it had
    // under its old name is gone from the project.
    let moved = files
        .iter()
        .map(|path| (format!("shop/{path}"), format!("store/{path}")))
        .collect::<Vec<_>>();
    let moved = moved
        .iter()
        .map(|(new, old)| (new.as_str(), old.as_str()))
        .collect::<Vec<_>>();
    let (store, renamed) = (
        shop("store", "billing", both),
        shop("shop", "billing", both),
    );
    assert_eq!(did(&store, &renamed, "shop", &moved), (1, 0, 0));
    assert_eq!(did(&store, &renamed, "", &moved), (2, 0, 0));
    assert_eq!(did(&store, &renamed, "shop", &[]), (1, 1, 0));
    assert_eq!(did(&store, &renamed, "", &[]), (2, 1, 1));
    // What the renamed folder no longer has against the order is gone from
    // it under its new name.
    let mended = shop("shop", "billing", (false, true));
    assert_eq!(did(&store, &mended, "shop", &moved), (0, 0, 1));
    assert_eq!(did(&store, &mended, "", &moved), (1, 0, 1));
    // A pair moved into the folder with what it had against the order is
    // no pair the change added there, and one moved out of it is none the
    // change took away: the project has as many as it had.
    let into = [
        ("store/Models/Invoice.php", "billing/Models/Invoice.php"),
        (
            "store/Services/InvoiceService.php",
            "billing/Services/InvoiceService.php",
        ),
    ];
    let out = [(into[0].1, into[0].0), (into[1].1, into[1].0)];
    let (apart, together) = (shop("store", "billing", both), shop("store", "store", both));
    assert_eq!(did(&apart, &together, "store", &into), (2, 0, 0));
    assert_eq!(did(&apart, &together, "", &into), (2, 0, 0));
    assert_eq!(did(&together, &apart, "store", &out), (1, 0, 0));
    assert_eq!(did(&together, &apart, "", &out), (2, 0, 0));
    // Unknown to be the same files, they are a pair added and a pair gone;
    // in the folder they left, a pair gone and none added.
    assert_eq!(did(&apart, &together, "store", &[]), (2, 1, 0));
    assert_eq!(did(&apart, &together, "", &[]), (2, 1, 1));
    assert_eq!(did(&together, &apart, "store", &[]), (1, 0, 1));
    // Moved out and mended there, the pair is gone from the project, and
    // not from the folder it had left.
    let left = shop("store", "billing", (true, false));
    assert_eq!(did(&together, &left, "store", &out), (1, 0, 0));
    assert_eq!(did(&together, &left, "", &out), (1, 0, 1));
    // What a change adds or takes away elsewhere is not of this folder.
    assert_eq!(did(&left, &apart, "store", &[]), (1, 0, 0));
    assert_eq!(did(&left, &apart, "", &[]), (2, 1, 0));
    assert_eq!(did(&apart, &left, "store", &[]), (1, 0, 0));
    assert_eq!(did(&apart, &left, "", &[]), (1, 0, 1));
    // Nor is a pair with one of its files outside the folder.
    let mut across = shop("store", "billing", (true, false));
    across.link(
        "billing/Models/Invoice.php",
        "store/Services/CheckoutService.php",
        "calls",
        None,
    );
    assert_eq!(did(&left, &across, "store", &[]), (1, 0, 0));
    assert_eq!(did(&left, &across, "", &[]), (2, 1, 0));
    assert_eq!(did(&across, &left, "store", &[]), (1, 0, 0));
    assert_eq!(did(&across, &left, "", &[]), (1, 0, 1));
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
