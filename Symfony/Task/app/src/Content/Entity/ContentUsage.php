<?php

declare(strict_types=1);

namespace App\Content\Entity;

use App\Content\Repository\ContentUsageRepository;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use Doctrine\ORM\Mapping as ORM;

/**
 * `trainer` here is the REUSING trainer, never the content's original
 * creator — architect-architecture.md "Entity population": "Scoped to the
 * *reusing* trainer... the creator's 'used by N trainers' figure is
 * therefore a crossing read, which is correct: it is cross-tenant
 * information." One row per (reusing trainer, content item) pair, upserted
 * the first time that trainer adds the item to any of their own playlists —
 * a second, third, ... addition of the same item to more of that trainer's
 * own playlists does not insert a second row.
 *
 * Built because the architecture requires it and AC-04-37 ("top content
 * creators") needs it — broad creator analytics beyond that stays
 * deliberately out of MVP scope, per the task brief.
 *
 * @see specs/database-designer-schema.md "`content_usage`"
 * @see specs/requirements-analyst-epic-04-lp-content-spec.md BR-04-12, AC-04-37
 */
#[ORM\Entity(repositoryClass: ContentUsageRepository::class)]
#[ORM\Table(name: 'content_usage')]
#[ORM\UniqueConstraint(name: 'uniq_content_usage_trainer_item', columns: ['trainer_id', 'content_item_id'])]
#[ORM\Index(name: 'idx_content_usage_content_item', columns: ['content_item_id'])]
#[TrainerScoped]
class ContentUsage
{
    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\ManyToOne(targetEntity: ContentItem::class)]
    #[ORM\JoinColumn(name: 'content_item_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private ContentItem $contentItem;

    #[ORM\Column(name: 'first_used_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $firstUsedAt;

    public function __construct(Trainer $trainer, ContentItem $contentItem, ?\DateTimeImmutable $firstUsedAt = null)
    {
        $this->trainer = $trainer;
        $this->contentItem = $contentItem;
        $this->firstUsedAt = $firstUsedAt ?? new \DateTimeImmutable();
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getTrainer(): Trainer
    {
        return $this->trainer;
    }

    public function getContentItem(): ContentItem
    {
        return $this->contentItem;
    }

    public function getFirstUsedAt(): \DateTimeImmutable
    {
        return $this->firstUsedAt;
    }
}
