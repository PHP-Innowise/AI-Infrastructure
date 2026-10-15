<?php

declare(strict_types=1);

namespace App\Scheduling\Entity;

use App\Identity\Entity\Account;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use App\Scheduling\Repository\AttendanceEditRepository;
use Doctrine\ORM\Mapping as ORM;

/**
 * History for a change made to an `AttendanceRecord` (BR-02-18). Ordinary
 * history table, not privilege-restricted like the token ledger or the
 * audit log — same-day coach edit permission is enforced at the service
 * layer, not by a database constraint (see AttendanceRecord's own note).
 *
 * @see specs/database-designer-schema.md "`attendance_edit`"
 */
#[ORM\Entity(repositoryClass: AttendanceEditRepository::class)]
#[ORM\Table(name: 'attendance_edit')]
#[ORM\Index(name: 'idx_attendance_edit_record', columns: ['attendance_record_id', 'edited_at'])]
#[TrainerScoped]
class AttendanceEdit
{
    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\ManyToOne(targetEntity: AttendanceRecord::class)]
    #[ORM\JoinColumn(name: 'attendance_record_id', referencedColumnName: 'id', nullable: false, onDelete: 'CASCADE')]
    private AttendanceRecord $attendanceRecord;

    #[ORM\Column(name: 'old_status', type: 'string', length: 16)]
    private string $oldStatus;

    #[ORM\Column(name: 'new_status', type: 'string', length: 16)]
    private string $newStatus;

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'edited_by_account_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Account $editedByAccount;

    #[ORM\Column(name: 'edited_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $editedAt;

    public function __construct(
        Trainer $trainer,
        AttendanceRecord $attendanceRecord,
        string $oldStatus,
        string $newStatus,
        Account $editedByAccount,
        \DateTimeImmutable $editedAt,
    ) {
        AttendanceRecord::guardStatus($oldStatus);
        AttendanceRecord::guardStatus($newStatus);

        $this->trainer = $trainer;
        $this->attendanceRecord = $attendanceRecord;
        $this->oldStatus = $oldStatus;
        $this->newStatus = $newStatus;
        $this->editedByAccount = $editedByAccount;
        $this->editedAt = $editedAt;
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getTrainer(): Trainer
    {
        return $this->trainer;
    }

    public function getAttendanceRecord(): AttendanceRecord
    {
        return $this->attendanceRecord;
    }

    public function getOldStatus(): string
    {
        return $this->oldStatus;
    }

    public function getNewStatus(): string
    {
        return $this->newStatus;
    }

    public function getEditedByAccount(): Account
    {
        return $this->editedByAccount;
    }

    public function getEditedAt(): \DateTimeImmutable
    {
        return $this->editedAt;
    }
}
