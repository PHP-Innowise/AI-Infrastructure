<?php

declare(strict_types=1);

namespace App\Identity\Entity;

use App\Identity\Repository\ShareLinkOpenRepository;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use Doctrine\ORM\Mapping as ORM;

/**
 * One row per click on a ShareLink's landing page. BR-01-27: usage timing,
 * not just a use count, is tracked per link (feeds Epic-06 analytics).
 *
 * @see specs/database-designer-schema.md "`share_link_open`"
 */
#[ORM\Entity(repositoryClass: ShareLinkOpenRepository::class)]
#[ORM\Table(name: 'share_link_open')]
#[ORM\Index(name: 'idx_share_link_open_link', columns: ['share_link_id', 'opened_at'])]
#[TrainerScoped]
class ShareLinkOpen
{
    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\ManyToOne(targetEntity: ShareLink::class)]
    #[ORM\JoinColumn(name: 'share_link_id', referencedColumnName: 'id', nullable: false, onDelete: 'CASCADE')]
    private ShareLink $shareLink;

    #[ORM\Column(name: 'opened_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $openedAt;

    #[ORM\Column(name: 'ip_address', type: 'string', length: 45, nullable: true)]
    private ?string $ipAddress = null;

    public function __construct(Trainer $trainer, ShareLink $shareLink, ?string $ipAddress = null)
    {
        $this->trainer = $trainer;
        $this->shareLink = $shareLink;
        $this->openedAt = new \DateTimeImmutable();
        $this->ipAddress = $ipAddress;
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getShareLink(): ShareLink
    {
        return $this->shareLink;
    }

    public function getOpenedAt(): \DateTimeImmutable
    {
        return $this->openedAt;
    }
}
