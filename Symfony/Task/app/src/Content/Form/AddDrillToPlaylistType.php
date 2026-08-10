<?php

declare(strict_types=1);

namespace App\Content\Form;

use App\Content\Entity\Drill;
use Symfony\Bridge\Doctrine\Form\Type\EntityType;
use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\Extension\Core\Type\TextareaType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;

/**
 * AC-04-8/AC-04-12: "Add to Playlist" — an existing drill (this trainer's
 * own, or a public one being reused, per BR-04-12), plus an optional
 * per-playlist trainer note (AC-04-12). No time-override field: the epic
 * mentions one (US-04.04 "an estimated time overriding the drill's
 * default") but neither its own Data Requirements nor the settled schema
 * (`playlist_item` has no such column) models it — see the coder's final
 * report.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class AddDrillToPlaylistType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('drill', EntityType::class, [
                'class' => Drill::class,
                'choices' => $options['candidates'],
                'choice_label' => static fn (Drill $d): string => $d->getTitle(),
            ])
            ->add('trainerNotes', TextareaType::class, ['required' => false])
            ->add('submit', SubmitType::class, ['label' => 'Add to playlist']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
        $resolver->setRequired(['candidates']);
    }
}
