# Symfony Clean Code Patterns

Illustrative review examples for the accelerator. Project conventions, installed Symfony/PHP versions, `AGENTS.md`, specifications, and tests take precedence. Snippets omit incidental imports and configuration so the architectural decision stays visible. Blocks labeled **Bad** are deliberately noncompliant counterexamples and must not be copied into production code.

## 1. Thin Controller, Explicit Use Case

Bad: the controller validates business rules, queries Doctrine, mutates state, and sends mail.

```php
final class CancelOrderController extends AbstractController
{
    #[Route('/orders/{id}/cancel', methods: ['POST'])]
    public function cancel(Request $request, Order $order): Response
    {
        if ($order->getUser() !== $this->getUser() || $order->isShipped()) {
            throw $this->createAccessDeniedException();
        }

        $order->cancel();
        $this->entityManager->flush();
        $this->mailer->send(/* ... */);

        return $this->redirectToRoute('order_show', ['id' => $order->getId()]);
    }
}
```

Good: the controller maps the framework boundary, authorizes, invokes one use case, and maps the result.

```php
final class CancelOrderController extends AbstractController
{
    #[Route('/orders/{id}/cancel', methods: ['POST'])]
    public function cancel(
        Request $request,
        Order $order,
        #[CurrentUser] User $actor,
        CancelOrder $cancelOrder,
    ): Response {
        if (!$this->isCsrfTokenValid('cancel-order-'.$order->getId(), (string) $request->request->get('_token'))) {
            throw $this->createAccessDeniedException('Invalid CSRF token.');
        }

        $this->denyAccessUnlessGranted(OrderVoter::CANCEL, $order);
        $cancelOrder($order->id(), $actor->id());

        return $this->redirectToRoute('order_show', ['id' => $order->getId()]);
    }
}
```

## 2. Cohesive Service And Transaction Boundary

Bad: a generic manager mixes unrelated use cases and exposes mode flags.

```php
final class OrderManager
{
    public function process(Order $order, bool $cancel, bool $notify, bool $refund): void
    {
        // Several unrelated workflows and invalid flag combinations.
    }
}
```

Good: one application service names one workflow and owns transaction/side-effect ordering.

```php
final readonly class CancelOrder
{
    public function __construct(
        private OrderRepository $orders,
        private TransactionManager $transactions,
        private OrderOutbox $outbox,
        private ClockInterface $clock,
    ) {
    }

    public function __invoke(OrderId $orderId, UserId $actorId): void
    {
        $this->transactions->run(function () use ($orderId, $actorId): void {
            $order = $this->orders->get($orderId);
            $order->cancel($this->clock->now());

            $this->outbox->append(new OrderCancelled($order->id(), $actorId));
        });
    }
}
```

`TransactionManager` and `OrderOutbox` are narrow infrastructure ports. The outbox implementation must persist with the order in the same transaction; a separate publisher can deliver the event after commit with retry and idempotency.

## 3. Dependency Inversion At A Real Boundary

Bad: the application service constructs infrastructure and depends on vendor details.

```php
final class SendReceipt
{
    public function __invoke(Order $order): void
    {
        $client = new VendorMailClient($_ENV['MAIL_TOKEN']);
        $client->send($order->customerEmail(), 'Receipt');
    }
}
```

Good: the use case depends on a narrow capability; the Symfony adapter owns vendor mapping.

```php
interface ReceiptSender
{
    public function send(OrderReceipt $receipt): void;
}

final readonly class SendReceipt
{
    public function __construct(private ReceiptSender $sender)
    {
    }

    public function __invoke(OrderReceipt $receipt): void
    {
        $this->sender->send($receipt);
    }
}

final readonly class SymfonyMailerReceiptSender implements ReceiptSender
{
    public function __construct(private MailerInterface $mailer)
    {
    }

    public function send(OrderReceipt $receipt): void
    {
        $this->mailer->send(/* map the receipt to an Email */);
    }
}
```

Do not add `OrderRepositoryInterface` beside one stable Doctrine repository merely to claim dependency inversion. Add an interface when substitution, package ownership, or a volatile external boundary makes the contract valuable.

## 4. Typed Input And Validation Boundary

Bad: untrusted arrays flow into an entity and clients can set privileged fields.

```php
$payload = $request->toArray();
$user->setEmail($payload['email']);
$user->setRoles($payload['roles']);
```

Good: the public write contract is typed and allowlisted; authorization remains separate from validation.

```php
final readonly class UpdateProfileInput
{
    public function __construct(
        #[Assert\NotBlank]
        #[Assert\Email]
        public string $email,
        #[Assert\Length(max: 120)]
        public string $displayName,
    ) {
    }
}

final class UpdateProfileController extends AbstractController
{
    #[Route('/profile', methods: ['PUT'])]
    public function update(
        #[MapRequestPayload] UpdateProfileInput $input,
        #[CurrentUser] User $actor,
        UpdateProfile $update,
    ): JsonResponse {
        $this->denyAccessUnlessGranted('PROFILE_EDIT');
        $profile = $update($actor->id(), $input);

        return $this->json(ProfileView::from($profile));
    }
}
```

Stateful uniqueness still needs a database unique constraint; a Validator check alone cannot close concurrency races.

## 5. Authorization And Voters

Bad: ownership is checked only in Twig or by hiding a button.

```twig
{% if app.user == order.customer %}
    <a href="{{ path('order_edit', {id: order.id}) }}">Edit</a>
{% endif %}
```

Good: the server authorizes the capability and the voter stays narrow.

```php
final class OrderVoter extends Voter
{
    public const EDIT = 'ORDER_EDIT';

    protected function supports(string $attribute, mixed $subject): bool
    {
        return self::EDIT === $attribute && $subject instanceof Order;
    }

    protected function voteOnAttribute(string $attribute, mixed $subject, TokenInterface $token): bool
    {
        $user = $token->getUser();

        return $user instanceof User
            && $subject->belongsTo($user->id())
            && $subject->isEditable();
    }
}
```

Collection endpoints also need repository/provider scoping; an item voter does not prevent cross-tenant collection exposure.

## 6. Doctrine Query Ownership And Parameters

Bad: a controller builds DQL with user input and leaks query mechanics.

```php
$term = $request->query->get('q');
$orders = $entityManager->createQuery("SELECT o FROM App\\Entity\\Order o WHERE o.reference LIKE '%$term%'")->getResult();
```

Good: the repository owns a bounded, parameterized query with stable ordering.

```php
final class OrderRepository extends ServiceEntityRepository
{
    /** @return list<Order> */
    public function searchForCustomer(CustomerId $customerId, string $term, int $limit = 50): array
    {
        return $this->createQueryBuilder('o')
            ->andWhere('o.customerId = :customerId')
            ->andWhere('o.reference LIKE :term')
            ->setParameter('customerId', $customerId->toString())
            ->setParameter('term', '%'.$term.'%')
            ->orderBy('o.createdAt', 'DESC')
            ->addOrderBy('o.id', 'DESC')
            ->setMaxResults(max(1, min($limit, 50)))
            ->getQuery()
            ->getResult();
    }
}
```

Back this query with an index matching tenant/filter/sort behavior and verify it with an explain plan on representative data.

## 7. Entity Invariants Without Framework Coupling

Bad: an entity sends mail, reads security context, or calls Doctrine.

```php
final class Order
{
    public function cancel(Security $security, MailerInterface $mailer): void
    {
        // Framework and side effects inside the entity.
    }
}
```

Good: the entity protects local state; the application service coordinates external effects.

```php
final class Order
{
    public function cancel(DateTimeImmutable $cancelledAt): void
    {
        if (!$this->status->canCancel()) {
            throw OrderCannotBeCancelled::forStatus($this->status);
        }

        $this->status = OrderStatus::Cancelled;
        $this->cancelledAt = $cancelledAt;
    }
}
```

The application service supplies time through a clock. The voter owns caller authorization; the entity owns only its local state transition. Do not pass the container, Security service, or clock service into the entity.

## 8. Messenger Message And Handler

Bad: the message contains a managed entity and the handler contains the workflow.

```php
final readonly class GenerateInvoice { public function __construct(public Order $order) {} }

final class GenerateInvoiceHandler
{
    public function __invoke(GenerateInvoice $message): void
    {
        // Query, calculate, persist, upload, and email inline.
    }
}
```

Good: the payload is stable and the handler delegates an idempotent use case.

```php
final readonly class GenerateInvoice
{
    public function __construct(public string $orderId)
    {
    }
}

#[AsMessageHandler]
final readonly class GenerateInvoiceHandler
{
    public function __construct(private GenerateInvoiceForOrder $generate)
    {
    }

    public function __invoke(GenerateInvoice $message): void
    {
        ($this->generate)(OrderId::fromString($message->orderId));
    }
}
```

Assume at-least-once delivery. Define duplicate handling, retryable failures, failure transport, payload compatibility, and dispatch timing relative to database commit.

## 9. Console And Event Adapters

Bad: commands and subscribers hide primary workflows.

```php
final class ExpireOrdersCommand extends Command
{
    protected function execute(InputInterface $input, OutputInterface $output): int
    {
        foreach ($this->entityManager->getRepository(Order::class)->findAll() as $order) {
            // Unbounded query and business logic.
        }

        return Command::SUCCESS;
    }
}
```

Good: adapters normalize input/event data and delegate.

```php
#[AsCommand(name: 'app:orders:expire')]
final class ExpireOrdersCommand extends Command
{
    public function __construct(private readonly ExpireOrders $expireOrders)
    {
        parent::__construct();
    }

    protected function execute(InputInterface $input, OutputInterface $output): int
    {
        $count = ($this->expireOrders)(new ExpireOrdersOptions(batchSize: 200));
        $output->writeln((string) $count);

        return Command::SUCCESS;
    }
}

final readonly class KernelExceptionSubscriber implements EventSubscriberInterface
{
    public function __construct(private ReportApplicationFailure $report)
    {
    }

    public function onKernelException(ExceptionEvent $event): void
    {
        ($this->report)($event->getThrowable());
    }

    public static function getSubscribedEvents(): array
    {
        return [KernelEvents::EXCEPTION => 'onKernelException'];
    }
}
```

Subscribers are appropriate for framework integration and cross-cutting observation. Use explicit service calls when business ordering or a return value matters.

## 10. API Platform Provider And Processor

Bad: expose writable entities broadly and let a processor own the whole use case.

```php
#[ApiResource(normalizationContext: ['groups' => ['user']], denormalizationContext: ['groups' => ['user']])]
final class User
{
    #[Groups(['user'])]
    public array $roles = [];
}
```

Good: define explicit contracts and delegate query/write behavior.

```php
#[Post(
    input: RegisterUserInput::class,
    output: UserView::class,
    processor: RegisterUserProcessor::class,
    security: "is_granted('USER_CREATE')",
)]
final class UserResource
{
}

final readonly class RegisterUserProcessor implements ProcessorInterface
{
    public function __construct(private RegisterUser $registerUser)
    {
    }

    public function process(mixed $data, Operation $operation, array $uriVariables = [], array $context = []): UserView
    {
        if (!$data instanceof RegisterUserInput) {
            throw new LogicException(sprintf('Expected %s, got %s.', RegisterUserInput::class, get_debug_type($data)));
        }

        return UserView::from(($this->registerUser)($data));
    }
}
```

Providers own collection scoping and query efficiency; processors coordinate through services. Review Serializer groups, filters, pagination, GraphQL, and Mercure independently.

## 11. Twig, Forms, And Progressive Enhancement

Bad: business decisions, raw output, and state-changing GET links live in Twig.

```twig
{% if order.total > 1000 and app.user %}
    {{ order.customerNote|raw }}
    <a href="{{ path('order_cancel', {id: order.id}) }}">Cancel</a>
{% endif %}
```

Good: the view receives decisions, output stays escaped, and mutation uses a CSRF-protected form.

```twig
{% if orderView.canCancel %}
    <form method="post" action="{{ path('order_cancel', {id: orderView.id}) }}">
        <input type="hidden" name="_token" value="{{ csrf_token('cancel-order-' ~ orderView.id) }}">
        <button type="submit">{{ 'order.cancel'|trans }}</button>
    </form>
{% endif %}

<p>{{ orderView.customerNote }}</p>
```

Stimulus and Turbo should enhance a working HTML flow. Preserve keyboard access, focus, validation errors, loading/error states, and behavior after Turbo reconnects.

## 12. Tests By Layer And Behavior

Bad: a controller test mocks Doctrine internals and only asserts status `200`.

```php
final class CancelOrderTest extends TestCase
{
    public function testCancel(): void
    {
        $this->entityManager->expects(self::once())->method('flush');
        self::assertSame(200, $this->client->request('POST', '/orders/1/cancel')->getStatusCode());
    }
}
```

Good: tests protect contracts at the layer that owns them.

```php
final class OrderTest extends TestCase
{
    public function testCustomerCanCancelPendingOrder(): void
    {
        $order = OrderBuilder::pending()->ownedBy($this->customerId)->build();
        $cancelledAt = new DateTimeImmutable('2026-07-15T12:00:00+00:00');

        $order->cancel($cancelledAt);

        self::assertSame(OrderStatus::Cancelled, $order->status());
        self::assertEquals($cancelledAt, $order->cancelledAt());
    }

    public function testShippedOrderCannotBeCancelled(): void
    {
        $order = OrderBuilder::shipped()->build();

        $this->expectException(OrderCannotBeCancelled::class);
        $order->cancel(new DateTimeImmutable('2026-07-15T12:00:00+00:00'));
    }
}
```

Add a service unit test for orchestration, repository integration test for custom queries/locking, voter tests for allow/deny/tenant cases, and a functional test for routing, validation, CSRF/auth, response contract, and persisted outcome. Mock stable external boundaries, not the code under test's internal implementation details.

## Review Questions

Use these questions instead of mechanically counting layers or interfaces:

1. Which class owns the business decision, transaction, query shape, authorization, and response mapping?
2. Do dependencies point from framework/infrastructure toward application behavior?
3. Is every abstraction justified by substitution, volatility, ownership, or a narrower consumer contract?
4. Are validation, authorization, database constraints, concurrency, and side-effect ordering explicit?
5. Can tests verify behavior without coupling to private implementation details?
6. Will operators understand retries, failure recovery, migration locks, cache behavior, and worker lifecycle?
