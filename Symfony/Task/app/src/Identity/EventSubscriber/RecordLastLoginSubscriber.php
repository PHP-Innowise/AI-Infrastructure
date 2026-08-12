<?php

declare(strict_types=1);

namespace App\Identity\EventSubscriber;

use App\Identity\Entity\Account;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Component\EventDispatcher\EventSubscriberInterface;
use Symfony\Component\Security\Http\Event\LoginSuccessEvent;

/**
 * Epic-07 § "Users tool" lists "Last login" as a column, and Epic-01 §
 * "Data Requirements" lists "Last login timestamp" as account data.
 * `account.last_login_at` existed, `Account::recordLogin()` existed, and
 * nothing ever called it — so the column read "Never" for every account,
 * including ones that had just signed in. Manual testing found it that way:
 * the requirement had been seen far enough to build a column and a setter,
 * and stopped one step short of a caller.
 *
 * `LoginSuccessEvent` is dispatched by `AuthenticatorManager` for every
 * authenticator that succeeds — the form login here, and anything added
 * later — which is what makes this the one place to record it rather than
 * something each authenticator has to remember.
 *
 * **Impersonation is deliberately not a login.** Symfony's `switch_user`
 * swaps the token through `SwitchUserListener`, which never goes through the
 * authenticator manager and never dispatches this event, so an admin viewing
 * a parent's portal does not rewrite that parent's last-login date. It would
 * be a quiet falsehood on the very screen an admin uses to judge whether an
 * account is still in use — and `impersonation_start` in the audit log is
 * where that visit genuinely belongs.
 *
 * @see specs/requirements-analyst-epic-07-super-admin-spec.md AC-07-10
 */
final readonly class RecordLastLoginSubscriber implements EventSubscriberInterface
{
    public function __construct(
        private EntityManagerInterface $entityManager,
    ) {
    }

    /**
     * @return array<string, string>
     */
    public static function getSubscribedEvents(): array
    {
        return [LoginSuccessEvent::class => 'onLoginSuccess'];
    }

    public function onLoginSuccess(LoginSuccessEvent $event): void
    {
        $account = $event->getUser();

        if (!$account instanceof Account) {
            return;
        }

        $account->recordLogin();
        $this->entityManager->flush();
    }
}
