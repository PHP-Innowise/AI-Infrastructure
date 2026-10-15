<?php

declare(strict_types=1);

namespace App\Tests\Platform;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Test\KernelTestCase;
use Symfony\Component\PasswordHasher\Hasher\PasswordHasherFactory;
use Symfony\Component\Yaml\Yaml;

/**
 * AC-01-75: all user data is stored securely — passwords hashed, emails
 * unique.
 *
 * Password hashing is checked against a hasher built the same way
 * config/packages/security.yaml configures the real (non-test)
 * `password_hashers: 'auto'` entry, rather than through the container's own
 * `UserPasswordHasherInterface` — that service resolves to the `when@test`
 * override (`algorithm: plaintext`), deliberately weakened so the suite is
 * not dominated by bcrypt (see that file's own comment). Asserting through
 * the test-env service would therefore either fail or pass for the wrong
 * reason; this test proves the mechanism actually configured for every
 * other environment does what AC-01-75 requires.
 *
 * @see specs/requirements-analyst-epic-01-user-management-spec.md AC-01-75, BR-01-2
 */
final class DataSecurityTest extends KernelTestCase
{
    /**
     * AC-01-75: passwords are hashed, not stored (or comparable) as plain
     * text, under the algorithm this application actually ships with.
     */
    public function testPasswordsAreHashedUnderTheRealAutoAlgorithm(): void
    {
        self::bootKernel();

        $projectDir = self::getContainer()->getParameter('kernel.project_dir');
        /** @var array{security: array{password_hashers: array<string, mixed>}} $config */
        $config = Yaml::parseFile($projectDir.'/config/packages/security.yaml');
        self::assertSame(
            'auto',
            $config['security']['password_hashers']['Symfony\Component\Security\Core\User\PasswordAuthenticatedUserInterface'],
            'The real (non-test) configuration must use a genuine adaptive hasher, not plaintext.',
        );

        $factory = new PasswordHasherFactory([
            Account::class => ['algorithm' => 'auto'],
        ]);
        $hasher = $factory->getPasswordHasher(Account::class);

        $hash = $hasher->hash('correct-horse-battery-staple');

        self::assertNotSame('correct-horse-battery-staple', $hash, 'AC-01-75: the stored value must not be the plain password.');
        self::assertTrue($hasher->verify($hash, 'correct-horse-battery-staple'), 'A correct password must still verify against its hash.');
        self::assertFalse($hasher->verify($hash, 'wrong-password'), 'An incorrect password must not verify.');
    }

    /**
     * AC-01-75/BR-01-2: email uniqueness is a database constraint, not only
     * an application-level pre-check — enforced even if a caller bypasses
     * every service and writes through the EntityManager directly.
     */
    public function testEmailUniquenessIsEnforcedAtTheDatabaseLevel(): void
    {
        self::bootKernel();

        /** @var EntityManagerInterface $em */
        $em = self::getContainer()->get(EntityManagerInterface::class);

        $first = new Account('duplicate-check@example.test', 'irrelevant-hash', AccountRole::Player);
        $em->persist($first);
        $em->flush();

        $second = new Account('duplicate-check@example.test', 'irrelevant-hash', AccountRole::Player);
        $em->persist($second);

        $this->expectException(\Doctrine\DBAL\Exception::class);
        $em->flush();
    }

    /**
     * BR-01-2: the CITEXT column makes uniqueness case-insensitive too — a
     * differently-cased duplicate is rejected the same way.
     */
    public function testEmailUniquenessIsCaseInsensitiveAtTheDatabaseLevel(): void
    {
        self::bootKernel();

        /** @var EntityManagerInterface $em */
        $em = self::getContainer()->get(EntityManagerInterface::class);

        $first = new Account('case-check@example.test', 'irrelevant-hash', AccountRole::Player);
        $em->persist($first);
        $em->flush();

        $second = new Account('CASE-CHECK@EXAMPLE.TEST', 'irrelevant-hash', AccountRole::Player);
        $em->persist($second);

        $this->expectException(\Doctrine\DBAL\Exception::class);
        $em->flush();
    }
}
