<?php

declare(strict_types=1);

namespace App\Content\Form;

use App\Content\Entity\Playlist;
use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\CheckboxType;
use Symfony\Component\Form\Extension\Core\Type\ChoiceType;
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
 * AC-04-11: title, description, filters, visibility. Drills are added
 * afterward via `content_trainer_playlist_drill_add` — see
 * `PlaylistService::createPracticePlaylist()`'s own docblock for why this
 * is a metadata-only create form rather than the API spec's literal
 * single-form "ordered drill list" framing.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class PracticePlaylistType extends AbstractType
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
            ->add('submit', SubmitType::class, ['label' => (string) $options['submitLabel']]);

        $builder->addEventListener(FormEvents::POST_SUBMIT, function (FormEvent $event): void {
            $form = $event->getForm();
            $data = $form->getData();

            if (\is_array($data) && true === ($data['isPublic'] ?? false) && Playlist::AUDIENCE_COACHES_ONLY === ($data['audience'] ?? null)) {
                $form->get('audience')->addError(new FormError('Public content cannot be coaches-only.'));
            }
        });
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults([
            'csrf_protection' => true,
            'submitLabel' => 'Create Practice Playlist',
        ]);
    }
}
