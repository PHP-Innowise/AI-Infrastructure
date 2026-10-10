<?php

declare(strict_types=1);

namespace App\Crm\Form;

use App\Crm\Entity\PlayerNote;
use App\Scheduling\Entity\Event;
use Symfony\Bridge\Doctrine\Form\Type\EntityType;
use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\Extension\Core\Type\TextareaType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;
use Symfony\Component\Validator\Constraints\Length;
use Symfony\Component\Validator\Constraints\NotBlank;

/**
 * AC-03-19 (general, no event) and AC-03-20 (per-event, tied to a specific
 * event's own "+ Add Note" row) — one Form, an optional `event` field the
 * controller supplies pre-selected-and-locked for the per-event entry point
 * and omits entirely for the general one, matching
 * `specs/api-designer-spec.md:530`'s single `crm_trainer_player_note_add`
 * route for both.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class AddNoteType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder->add('text', TextareaType::class, [
            'constraints' => [new NotBlank(), new Length(max: PlayerNote::MAX_TEXT_LENGTH)],
        ]);

        if (null !== $options['events']) {
            $builder->add('event', EntityType::class, [
                'class' => Event::class,
                'choices' => $options['events'],
                'choice_label' => 'title',
                'required' => false,
                'placeholder' => 'General note (not tied to an event)',
            ]);
        }

        $builder->add('submit', SubmitType::class, ['label' => 'Add note']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true, 'events' => null]);
    }
}
