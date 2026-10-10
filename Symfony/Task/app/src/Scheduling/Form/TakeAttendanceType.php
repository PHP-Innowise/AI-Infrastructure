<?php

declare(strict_types=1);

namespace App\Scheduling\Form;

use App\Scheduling\Entity\AttendanceRecord;
use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\ChoiceType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;

/**
 * AC-02-38/39: one status choice per RSVP'd player, built dynamically from
 * the event's own roster — the same "known only at render time" shape
 * CoachAvailabilityController's dynamic slot fields use, generalized to a
 * roster of arbitrary size instead of a fixed 7 days.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class TakeAttendanceType extends AbstractType
{
    private const CHOICES = [
        'Present' => AttendanceRecord::STATUS_PRESENT,
        'Absent' => AttendanceRecord::STATUS_ABSENT,
        'Late' => AttendanceRecord::STATUS_LATE,
        'Excused' => AttendanceRecord::STATUS_EXCUSED,
    ];

    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        /** @var array<int, string> $players player id => display label */
        $players = $options['players'];
        /** @var array<int, string> $current player id => current status, defaults to Present */
        $current = $options['current'];

        foreach ($players as $playerId => $label) {
            $builder->add('player_'.$playerId, ChoiceType::class, [
                'label' => $label,
                'choices' => self::CHOICES,
                'data' => $current[$playerId] ?? AttendanceRecord::STATUS_PRESENT,
                'expanded' => true,
            ]);
        }

        $builder->add('submit', SubmitType::class, ['label' => 'Save attendance']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true, 'current' => []]);
        $resolver->setRequired(['players']);
    }
}
