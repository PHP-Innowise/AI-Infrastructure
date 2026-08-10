<?php

declare(strict_types=1);

namespace App\Content\Form;

use App\Content\Entity\Playlist;
use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\CheckboxType;
use Symfony\Component\Form\Extension\Core\Type\ChoiceType;
use Symfony\Component\Form\Extension\Core\Type\CollectionType;
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

/**
 * AC-04-1..3: title, description, customization filters, visibility, and an
 * embedded ordered list of videos (each validated by `VideoItemType`) — one
 * atomic submission, matching `specs/api-designer-spec.md`'s literal
 * `LearnPlaylistType` request shape ("title, description, filters,
 * visibility, ordered video list").
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class LearnPlaylistType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('title', TextType::class, ['constraints' => [new NotBlank(), new Length(max: Playlist::MAX_TITLE_LENGTH)]])
            ->add('description', TextareaType::class, ['required' => false])
            ->add('filterSkillLevels', TextType::class, ['required' => false, 'help' => 'Comma-separated'])
            ->add('filterPositions', TextType::class, ['required' => false, 'help' => 'Comma-separated'])
            ->add('filterAgeLevels', TextType::class, ['required' => false, 'help' => 'Comma-separated'])
            ->add('isPublic', CheckboxType::class, ['required' => false, 'label' => 'Make Public'])
            ->add('audience', ChoiceType::class, [
                'choices' => ['Players & Coaches' => Playlist::AUDIENCE_PLAYERS_AND_COACHES, 'Coaches Only' => Playlist::AUDIENCE_COACHES_ONLY],
            ])
            ->add('videos', CollectionType::class, [
                'entry_type' => VideoItemType::class,
                'allow_add' => true,
                'allow_delete' => true,
                'by_reference' => false,
                'prototype' => true,
            ])
            ->add('submit', SubmitType::class, ['label' => (string) $options['submitLabel']]);

        // AC-04-3: non-empty title (NotBlank above) and at least one video;
        // A9/AC-04-41: public + coaches-only is never a valid combination —
        // Playlist::setVisibility() enforces this too, but checking here
        // turns it into an ordinary form error instead of a raw exception
        // surfacing as a 500 (same reasoning as EventType's own POST_SUBMIT
        // listener).
        $builder->addEventListener(FormEvents::POST_SUBMIT, function (FormEvent $event): void {
            $form = $event->getForm();
            $data = $form->getData();

            if (!\is_array($data)) {
                return;
            }

            $videos = $data['videos'] ?? [];

            if ([] === $videos) {
                $form->get('videos')->addError(new FormError('At least one video is required.'));
            }

            if (true === ($data['isPublic'] ?? false) && Playlist::AUDIENCE_COACHES_ONLY === ($data['audience'] ?? null)) {
                $form->get('audience')->addError(new FormError('Public content cannot be coaches-only.'));
            }
        });
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults([
            'csrf_protection' => true,
            'submitLabel' => 'Create Learn Playlist',
        ]);
    }
}
