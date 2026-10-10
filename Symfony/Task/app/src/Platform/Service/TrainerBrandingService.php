<?php

declare(strict_types=1);

namespace App\Platform\Service;

use App\Platform\Entity\Trainer;
use App\Platform\Entity\TrainerBrandingSettings;
use App\Platform\Repository\TrainerBrandingSettingsRepository;
use Doctrine\ORM\EntityManagerInterface;

/**
 * US-01.14: logo + primary colour, MVP scope only (AC-01-63). Applies
 * immediately: branding is read fresh per request by `base.html.twig`'s
 * branding block, never cached, so "saving branding changes applies them
 * immediately for all users in the trainer's organization" (AC-01-62) is
 * true simply because there is nothing to invalidate.
 *
 * @see specs/requirements-analyst-epic-01-user-management-spec.md US-01.14, AC-01-60..63
 */
final readonly class TrainerBrandingService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private TrainerBrandingSettingsRepository $repository,
    ) {
    }

    public function getOrCreate(Trainer $trainer): TrainerBrandingSettings
    {
        $settings = $this->repository->findForTrainer($trainer);

        if (null === $settings) {
            $settings = new TrainerBrandingSettings($trainer);
            $this->repository->add($settings);
        }

        return $settings;
    }

    /**
     * AC-01-60: logo upload. `$logoPath` is a path already written under
     * public/uploads by the controller (multipart handling is an HTTP-layer
     * concern); this method only records where it landed.
     */
    public function updateLogo(Trainer $trainer, ?string $logoPath): void
    {
        $this->entityManager->wrapInTransaction(function () use ($trainer, $logoPath): void {
            $this->getOrCreate($trainer)->updateLogo($logoPath);
        });
    }

    /**
     * AC-01-61: primary colour, with reset-to-default.
     */
    public function updatePrimaryColor(Trainer $trainer, ?string $hex): void
    {
        $this->entityManager->wrapInTransaction(function () use ($trainer, $hex): void {
            $settings = $this->getOrCreate($trainer);

            if (null === $hex) {
                $settings->resetPrimaryColor();
            } else {
                $settings->updatePrimaryColor($hex);
            }
        });
    }
}
