<?php

declare(strict_types=1);

namespace App\DataFixtures;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountProfile;
use App\Identity\Entity\AccountRole;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Entity\PlayerTrainerMembership;
use App\Platform\Entity\AccountTrainerLink;
use App\Platform\Entity\PublicTenantCode;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TenantContext;
use Doctrine\Bundle\FixturesBundle\Fixture;
use Doctrine\Persistence\ObjectManager;
use Symfony\Component\PasswordHasher\Hasher\UserPasswordHasherInterface;

/**
 * A reviewer needs to log in as each of the four MVP roles, and needs a second
 * trainer to exist so cross-tenant isolation is demonstrable rather than
 * theoretical.
 *
 * Every password is `password`. This is development seed data and is never
 * loaded outside dev/test.
 */
final class AppFixtures extends Fixture
{
    public function __construct(
        private readonly UserPasswordHasherInterface $passwordHasher,
        private readonly TenantContext $tenantContext,
    ) {
    }

    public function load(ObjectManager $manager): void
    {
        $superAdmin = $this->account($manager, 'admin@practiceperfect.test', AccountRole::SuperAdmin, 'Dale', 'Administrator');

        // --- Tenant A ---------------------------------------------------------
        $trainerAccountA = $this->account($manager, 'trainer@practiceperfect.test', AccountRole::Trainer, 'Tina', 'Trainer');
        $trainerA = new Trainer($trainerAccountA, 'Peak Performance Basketball', 'peak-performance');
        $manager->persist($trainerA);

        // --- Tenant B, so isolation has something to isolate from -------------
        $trainerAccountB = $this->account($manager, 'trainer-b@practiceperfect.test', AccountRole::Trainer, 'Bruno', 'Baseline');
        $trainerB = new Trainer($trainerAccountB, 'Baseline Athletics', 'baseline-athletics');
        $manager->persist($trainerB);

        $coachAccount = $this->account($manager, 'coach@practiceperfect.test', AccountRole::Coach, 'Casey', 'Coach');
        $playerAccount = $this->account($manager, 'player@practiceperfect.test', AccountRole::Player, 'Pat', 'Parent');

        $manager->flush();

        // Links are what the resolver reads; they are global on purpose.
        $manager->persist(new AccountTrainerLink($trainerAccountA, $trainerA, 'trainer'));
        $manager->persist(new AccountTrainerLink($trainerAccountB, $trainerB, 'trainer'));
        $manager->persist(new AccountTrainerLink($coachAccount, $trainerA, 'coach'));
        $manager->persist(new AccountTrainerLink($playerAccount, $trainerA, 'player'));

        // A ShareLink code for tenant A, resolvable with no authentication.
        $manager->persist(new PublicTenantCode('join-peak-performance', $trainerA, PublicTenantCode::KIND_SHARELINK, 1));

        $childA = new PlayerProfile('Alex', new \DateTimeImmutable('2012-04-18'));
        $childA->markAsChild();
        $manager->persist($childA);

        $childB = new PlayerProfile('Blake', new \DateTimeImmutable('2011-09-02'));
        $childB->markAsChild();
        $manager->persist($childB);

        $manager->flush();

        // Trainer-scoped rows must be written under the tenant they belong to:
        // the RLS WITH CHECK clause rejects anything else. Writing one row per
        // tenant here is also what makes the isolation test meaningful.
        $this->tenantContext->activateFor($trainerA);
        $manager->persist(new PlayerTrainerMembership($trainerA, $childA, PlayerTrainerMembership::SOURCE_SHARELINK));
        $manager->flush();

        $this->tenantContext->activateFor($trainerB);
        $manager->persist(new PlayerTrainerMembership($trainerB, $childB, PlayerTrainerMembership::SOURCE_COACH_INVITE));
        $manager->flush();

        $this->tenantContext->clear();

        unset($superAdmin);
    }

    private function account(
        ObjectManager $manager,
        string $email,
        AccountRole $role,
        string $firstName,
        string $lastName,
    ): Account {
        $account = new Account($email, '', $role);
        $account->changePasswordHash($this->passwordHasher->hashPassword($account, 'password'));
        $account->verifyEmail();

        $manager->persist($account);
        $manager->persist(new AccountProfile($account, $firstName, $lastName));

        return $account;
    }
}
