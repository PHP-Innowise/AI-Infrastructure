<?php

declare(strict_types=1);

namespace App\Content\Form;

use App\Content\Entity\PlaylistAssignment;
use App\Crm\Entity\Label;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Entity\SkillLevel;
use Symfony\Bridge\Doctrine\Form\Type\EntityType;
use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\ChoiceType;
use Symfony\Component\Form\Extension\Core\Type\DateType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\Extension\Core\Type\TextType;
use Symfony\Component\Form\Extension\Core\Type\TextareaType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\Form\FormError;
use Symfony\Component\Form\FormEvent;
use Symfony\Component\Form\FormEvents;
use Symfony\Component\OptionsResolver\OptionsResolver;

/**
 * AC-04-13/BR-04-13: individual multi-select, by label group, or by a
 * skill-level filter — one of the three, matching `PlaylistAssignment`'s own
 * "exactly one target" invariant. `targetPlayers` is `multiple => true`
 * (AC-04-13's "selected count shown") even though `PlaylistAssignment`
 * itself carries one target row per act: the controller creates one row per
 * selected player when `targetType` is `player`, matching BR-04-14 "the same
 * playlist can be assigned... multiple times, with each assignment tracked
 * separately."
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class AssignPlaylistType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('targetType', ChoiceType::class, [
                'choices' => [
                    'Individual players' => PlaylistAssignment::TARGET_PLAYER,
                    'Label group' => PlaylistAssignment::TARGET_LABEL,
                    'Skill level' => PlaylistAssignment::TARGET_SKILL_LEVEL,
                ],
            ])
            ->add('targetPlayers', EntityType::class, [
                'class' => PlayerProfile::class,
                'choices' => $options['candidatePlayers'],
                'choice_label' => 'firstName',
                'multiple' => true,
                'required' => false,
            ])
            ->add('targetLabel', EntityType::class, [
                'class' => Label::class,
                'choices' => $options['candidateLabels'],
                'choice_label' => 'name',
                'required' => false,
                'placeholder' => 'Select a label',
            ])
            ->add('targetSkillLevel', ChoiceType::class, [
                'required' => false,
                'choices' => SkillLevel::choices(),
                'placeholder' => 'Choose a skill level',
                // AC-04-14 targets players BY skill level, so it has to name
                // the same four a profile can hold. Typed free-hand it
                // reached nobody whose profile spelled it differently.
                'label' => 'Skill level',
            ])
            ->add('dueDate', DateType::class, ['required' => false, 'widget' => 'single_text', 'input' => 'datetime_immutable'])
            ->add('note', TextareaType::class, ['required' => false, 'help' => 'Visible to the player'])
            ->add('submit', SubmitType::class, ['label' => 'Assign']);

        $builder->addEventListener(FormEvents::POST_SUBMIT, function (FormEvent $event): void {
            $form = $event->getForm();
            $data = $form->getData();

            if (!\is_array($data)) {
                return;
            }

            $targetType = $data['targetType'] ?? null;
            $players = $data['targetPlayers'] ?? [];
            $playerCount = $players instanceof \Countable || \is_array($players) ? \count($players) : 0;

            if (PlaylistAssignment::TARGET_PLAYER === $targetType && 0 === $playerCount) {
                $form->get('targetPlayers')->addError(new FormError('Select at least one player.'));
            }

            if (PlaylistAssignment::TARGET_LABEL === $targetType && null === ($data['targetLabel'] ?? null)) {
                $form->get('targetLabel')->addError(new FormError('Select a label.'));
            }

            if (PlaylistAssignment::TARGET_SKILL_LEVEL === $targetType && '' === trim((string) ($data['targetSkillLevel'] ?? ''))) {
                $form->get('targetSkillLevel')->addError(new FormError('Enter a skill level.'));
            }
        });
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
        $resolver->setRequired(['candidatePlayers', 'candidateLabels']);
    }
}
