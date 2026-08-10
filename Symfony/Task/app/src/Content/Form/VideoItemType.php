<?php

declare(strict_types=1);

namespace App\Content\Form;

use App\Content\Entity\ContentItem;
use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\IntegerType;
use Symfony\Component\Form\Extension\Core\Type\TextareaType;
use Symfony\Component\Form\Extension\Core\Type\TextType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;
use Symfony\Component\Validator\Constraints\Length;
use Symfony\Component\Validator\Constraints\NotBlank;
use Symfony\Component\Validator\Constraints\Regex;

/**
 * AC-04-2: one video row embedded in `LearnPlaylistType`'s collection.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class VideoItemType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('youtubeUrl', TextType::class, [
                'constraints' => [
                    new NotBlank(),
                    new Regex(pattern: ContentItem::YOUTUBE_URL_PATTERN, message: 'Enter a valid YouTube URL.'),
                ],
            ])
            ->add('title', TextType::class, ['constraints' => [new NotBlank(), new Length(max: ContentItem::MAX_TITLE_LENGTH)]])
            ->add('instructions', TextareaType::class, ['required' => false, 'constraints' => [new Length(max: ContentItem::MAX_INSTRUCTIONS_LENGTH)]])
            ->add('tags', TextType::class, ['required' => false, 'help' => 'Comma-separated'])
            ->add('durationSeconds', IntegerType::class, ['required' => false, 'help' => 'Seconds — auto-detected when available, or enter manually']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        // Nested inside LearnPlaylistType's own CSRF-protected form — a
        // second token here would be both redundant and awkward to render
        // per collection row.
        $resolver->setDefaults(['csrf_protection' => false]);
    }
}
