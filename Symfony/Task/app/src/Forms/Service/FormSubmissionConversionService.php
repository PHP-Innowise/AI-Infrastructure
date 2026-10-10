<?php

declare(strict_types=1);

namespace App\Forms\Service;

use App\Billing\Entity\PaymentRecord;
use App\Forms\Entity\FormSubmission;
use App\Identity\Entity\Account;
use App\Identity\Exception\DuplicateEmailException;
use App\Identity\Repository\AccountRepository;
use App\Identity\Service\PlayerRegistrationService;
use Doctrine\ORM\EntityManagerInterface;

/**
 * US-08.05: converts a camp/evaluation `FormSubmission` into a full account.
 * The account/player/membership creation itself is `Identity`'s job
 * (`PlayerRegistrationService::registerViaCampConversion()` — see that
 * method's own docblock for why); this class owns only what is genuinely
 * Forms' own concern: the pre-check that avoids a duplicate account
 * (AC-08-26), and A5's own follow-up once Identity's transaction has
 * committed — marking the submission converted and reattaching its earlier
 * payment record to the new account.
 *
 * @see specs/requirements-analyst-open-questions.md "A5"
 * @see specs/requirements-analyst-epic-08-forms-registration-spec.md US-08.05, AC-08-23..26
 */
final readonly class FormSubmissionConversionService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private AccountRepository $accounts,
        private PlayerRegistrationService $playerRegistration,
    ) {
    }

    /**
     * AC-08-26: checked before the conversion form is even rendered — a
     * submission whose email already has an account never sees the form at
     * all (api-designer-spec.md: "rather than rendering the form at all").
     */
    public function accountAlreadyExists(FormSubmission $submission): bool
    {
        return null !== $this->accounts->findOneByEmail($submission->getContactEmail());
    }

    /**
     * @throws DuplicateEmailException a race: the email gained an account
     *                                 between the GET check above and this POST — AC-08-26 still applies,
     *                                 the caller re-renders the same "log in instead" outcome rather than
     *                                 creating a duplicate
     */
    public function convert(FormSubmission $submission, \DateTimeImmutable $dateOfBirth, ?string $gender, string $plainPassword): Account
    {
        if ($submission->isConverted()) {
            throw new \LogicException('This submission has already been converted.');
        }

        $account = $this->playerRegistration->registerViaCampConversion(
            $submission->getForm()->getTrainer(),
            $submission->participantName(),
            $submission->getContactEmail(),
            $plainPassword,
            $dateOfBirth,
            $gender,
        );

        // A5: the earlier camp payment and registration attach to the new
        // account — a plain UPDATE on the existing PaymentRecord row, never
        // a new one (PaymentRecord is mutable; see its own docblock).
        $this->entityManager->wrapInTransaction(function () use ($submission, $account): void {
            $submission->markConverted($account);

            $paymentRecord = $submission->getPaymentRecord();

            if (null !== $paymentRecord && PaymentRecord::TYPE_CAMP_REGISTRATION === $paymentRecord->getType()) {
                $paymentRecord->attachPayerAccount($account);
            }

            $this->entityManager->flush();
        });

        return $account;
    }
}
