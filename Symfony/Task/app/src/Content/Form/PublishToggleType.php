<?php

declare(strict_types=1);

namespace App\Content\Form;

use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\ChoiceType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;

/**
 * AC-04-17/18: a drill's plain public/private toggle — `ContentItem`/`Drill`
 * carry no coach-only audience concept (that is a `Playlist`-level field
 * only, AC-04-41's own scope note), so this is simpler than
 * `PlaylistVisibilityType`.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class PublishToggleType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('isPublic', ChoiceType::class, ['choices' => ['Public' => true, 'Private' => false]])
            ->add('submit', SubmitType::class, ['label' => 'Save visibility']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
    }
}
