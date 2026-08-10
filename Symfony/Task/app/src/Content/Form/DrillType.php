<?php

declare(strict_types=1);

namespace App\Content\Form;

use App\Content\Entity\ContentItem;
use App\Content\Entity\Drill;
use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\ChoiceType;
use Symfony\Component\Form\Extension\Core\Type\IntegerType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\Extension\Core\Type\TextareaType;
use Symfony\Component\Form\Extension\Core\Type\TextType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\Form\FormError;
use Symfony\Component\Form\FormEvent;
use Symfony\Component\Form\FormEvents;
use Symfony\Component\OptionsResolver\OptionsResolver;
use Symfony\Component\Validator\Constraints\Length;
use Symfony\Component\Validator\Constraints\NotBlank;
use Symfony\Component\Validator\Constraints\Regex;

/**
 * AC-04-4..6: name, instructions, YouTube URL, difficulty, equipment, space,
 * player count, duration range, categories, visibility. `categories` is
 * required (AC-04-6); `equipment` is free-form comma-separated, matching
 * `EventType`'s own precedent for un-vocabularied array fields
 * (equipment/space/categories carry no fixed platform-wide vocabulary in
 * the settled schema — `TEXT[]`, not a CHECK-constrained enum).
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class DrillType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('title', TextType::class, ['constraints' => [new NotBlank(), new Length(max: ContentItem::MAX_TITLE_LENGTH)]])
            ->add('youtubeUrl', TextType::class, [
                'constraints' => [new NotBlank(), new Regex(pattern: ContentItem::YOUTUBE_URL_PATTERN, message: 'Enter a valid YouTube URL.')],
            ])
            ->add('instructions', TextareaType::class, ['required' => false, 'constraints' => [new Length(max: ContentItem::MAX_INSTRUCTIONS_LENGTH)]])
            ->add('tags', TextType::class, ['required' => false, 'help' => 'Comma-separated'])
            ->add('difficultyLevel', ChoiceType::class, ['choices' => array_combine(
                array_map(static fn (string $d): string => ucfirst($d), Drill::difficultyLevels()),
                Drill::difficultyLevels(),
            )])
            ->add('categories', TextType::class, ['constraints' => [new NotBlank()], 'help' => 'Comma-separated, e.g. Dribbling, Shooting'])
            ->add('equipment', TextType::class, ['required' => false, 'help' => 'Comma-separated, e.g. Ball, Cones'])
            ->add('spaceRequirement', ChoiceType::class, [
                'required' => false,
                'placeholder' => 'Not specified',
                'choices' => array_combine(array_map(static fn (string $s): string => ucfirst($s), Drill::spaceRequirements()), Drill::spaceRequirements()),
            ])
            ->add('playerCount', TextType::class, ['required' => false, 'help' => 'e.g. "1-2", "2-4"'])
            ->add('durationMinMinutes', IntegerType::class, ['required' => false])
            ->add('durationMaxMinutes', IntegerType::class, ['required' => false])
            ->add('isPublic', ChoiceType::class, ['choices' => ['Private' => false, 'Public' => true]])
            ->add('submit', SubmitType::class, ['label' => (string) $options['submitLabel']]);

        // AC-04-6: at least one non-blank category.
        $builder->addEventListener(FormEvents::POST_SUBMIT, function (FormEvent $event): void {
            $form = $event->getForm();
            $data = $form->getData();

            if (!\is_array($data)) {
                return;
            }

            $categories = array_filter(array_map('trim', explode(',', (string) ($data['categories'] ?? ''))), static fn (string $c): bool => '' !== $c);

            if ([] === $categories) {
                $form->get('categories')->addError(new FormError('At least one category is required.'));
            }

            $min = $data['durationMinMinutes'] ?? null;
            $max = $data['durationMaxMinutes'] ?? null;

            if (null !== $min && null !== $max && (int) $max < (int) $min) {
                $form->get('durationMaxMinutes')->addError(new FormError('Maximum duration cannot be less than minimum duration.'));
            }
        });
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults([
            'csrf_protection' => true,
            'submitLabel' => 'Save drill',
        ]);
    }
}
