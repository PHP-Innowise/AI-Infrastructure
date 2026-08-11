<?php

declare(strict_types=1);

namespace App\Content\Form;

use App\Content\Entity\Playlist;
use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\IntegerType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\Extension\Core\Type\TextareaType;
use Symfony\Component\Form\Extension\Core\Type\TextType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;
use Symfony\Component\Validator\Constraints\GreaterThanOrEqual;
use Symfony\Component\Validator\Constraints\Length;
use Symfony\Component\Validator\Constraints\NotBlank;

/**
 * AC-04-31: title, description, and customization filters. Deliberately its
 * own form, distinct from `LearnPlaylistType`/`PracticePlaylistType` —
 * `specs/api-designer-spec.md`'s route table literally reuses those two for
 * `content_trainer_playlist_edit`, but their embedded video collection and
 * visibility fields do not fit an edit flow cleanly: visibility already has
 * its own dedicated route/form (`PlaylistVisibilityType`,
 * `content_trainer_playlist_visibility`), and adding/removing items already
 * has its own routes (`content_trainer_playlist_drill_add`,
 * `content_trainer_playlist_item_remove`) plus drag-and-drop reordering
 * (`content_trainer_playlist_items_reorder`) — reusing the create forms
 * here would mean re-diffing a submitted video collection against existing
 * `PlaylistItem` rows with no stable correlation key, solving the same
 * problem those three routes already solve, worse. Recorded in the coder's
 * final report as a deliberate deviation from the literal route table.
 *
 * `priceUsdMinorUnits`/`priceTokens` (AC-05-18, Epic-05) are added to this
 * same edit form, matching how `Event`'s own dual pricing lives on its one
 * create/edit form (Epic-02) rather than a separate Billing-owned screen —
 * one playlist, edited in one place, price included.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class PlaylistEditType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('title', TextType::class, ['constraints' => [new NotBlank(), new Length(max: Playlist::MAX_TITLE_LENGTH)]])
            ->add('description', TextareaType::class, ['required' => false])
            ->add('filterSkillLevels', TextType::class, ['required' => false, 'help' => 'Comma-separated'])
            ->add('filterPositions', TextType::class, ['required' => false, 'help' => 'Comma-separated'])
            ->add('filterAgeLevels', TextType::class, ['required' => false, 'help' => 'Comma-separated'])
            ->add('priceUsdMinorUnits', IntegerType::class, ['label' => 'Price (USD cents)', 'constraints' => [new GreaterThanOrEqual(0)]])
            ->add('priceTokens', IntegerType::class, ['label' => 'Price (tokens)', 'constraints' => [new GreaterThanOrEqual(0)]])
            ->add('submit', SubmitType::class, ['label' => 'Save changes']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
    }
}
