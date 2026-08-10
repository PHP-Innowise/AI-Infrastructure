<?php

declare(strict_types=1);

namespace App\Crm\Form;

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
 * AC-03-46: a coach selects one of their recent shared sessions and writes
 * up to 500 characters — a tighter limit than a trainer's own per-event note
 * (1000 chars, AddNoteType), even though both write the same `PlayerNote`
 * row shape (see that entity's own docblock).
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class SessionFeedbackType extends AbstractType
{
    public const MAX_TEXT_LENGTH = 500;

    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('event', EntityType::class, [
                'class' => Event::class,
                'choices' => $options['recentSessions'],
                'choice_label' => 'title',
                'constraints' => [new NotBlank()],
            ])
            ->add('text', TextareaType::class, [
                'constraints' => [new NotBlank(), new Length(max: self::MAX_TEXT_LENGTH)],
            ])
            ->add('submit', SubmitType::class, ['label' => 'Add feedback']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
        $resolver->setRequired(['recentSessions']);
    }
}
