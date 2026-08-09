<?php

declare(strict_types=1);

/**
 * Boots an EntityManager for PHPStan's Doctrine extension.
 *
 * Without this, PHPStan cannot see that Doctrine assigns entity identifiers
 * and reads mapped properties through reflection, and reports every `$id`,
 * `$createdAt` and `$updatedAt` as written-but-never-read.
 *
 * @see phpstan.dist.neon
 */

use App\Kernel;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Component\Dotenv\Dotenv;

require dirname(__DIR__).'/vendor/autoload.php';

(new Dotenv())->bootEnv(dirname(__DIR__).'/.env');

$kernel = new Kernel((string) ($_SERVER['APP_ENV'] ?? 'dev'), (bool) ($_SERVER['APP_DEBUG'] ?? true));
$kernel->boot();

/** @var EntityManagerInterface $entityManager */
$entityManager = $kernel->getContainer()->get('doctrine')->getManager();

return $entityManager;
