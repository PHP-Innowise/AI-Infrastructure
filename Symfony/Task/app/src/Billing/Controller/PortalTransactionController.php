<?php

declare(strict_types=1);

namespace App\Billing\Controller;

use App\Billing\Repository\PaymentRecordRepository;
use App\Identity\Entity\Account;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * AC-05-22/23, BR-05-17: transaction history, strictly scoped to the
 * current trainer context AND the acting account — "a trainer cannot see a
 * player's transactions with a different trainer," and, symmetrically here,
 * a player never sees a different trainer's transactions in this context
 * either (both directions of BR-05-17 hold simultaneously: this query
 * filters on both `payerAccount = actor` AND runs inside the active
 * tenant's own RLS scope).
 *
 * @see specs/api-designer-spec.md "Billing module" — player/parent portal table
 */
#[IsGranted('ROLE_PLAYER')]
final class PortalTransactionController extends AbstractController
{
    public function __construct(
        private readonly PaymentRecordRepository $paymentRecords,
    ) {
    }

    #[Route('/portal/transactions', name: 'billing_portal_transactions', methods: ['GET'])]
    public function index(Request $request): Response
    {
        /** @var Account $actor */
        $actor = $this->getUser();

        $from = $this->parseDate($request->query->get('from'));
        $to = $this->parseDate($request->query->get('to'));
        $type = $request->query->get('type');
        $method = $request->query->get('method');

        $transactions = $this->paymentRecords->findForPayerHistory(
            $actor,
            $from,
            $to,
            null !== $type && '' !== $type ? [$type] : null,
            \is_string($method) && '' !== $method ? $method : null,
        );

        return $this->render('billing/portal_transactions.html.twig', [
            'transactions' => $transactions,
            'filters' => ['from' => $request->query->get('from'), 'to' => $request->query->get('to'), 'type' => $type, 'method' => $method],
        ]);
    }

    private function parseDate(mixed $value): ?\DateTimeImmutable
    {
        if (!\is_string($value) || '' === $value) {
            return null;
        }

        try {
            return new \DateTimeImmutable($value);
        } catch (\Exception) {
            return null;
        }
    }
}
