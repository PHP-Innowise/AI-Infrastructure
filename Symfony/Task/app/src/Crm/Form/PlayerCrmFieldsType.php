<?php

declare(strict_types=1);

namespace App\Crm\Form;

use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\Extension\Core\Type\TextType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;

/**
 * AC-03-33: "the trainer can edit the player profile's limited fields
 * (skill level, notes)" — "notes" here is the dedicated Notes section
 * (AddNoteType), not a form field on this one, so `skillLevel` is the only
 * field this Form owns. Free text, matching `player_trainer_membership.skill_level`'s
 * own VARCHAR(50) — see Q-01.01 in `specs/requirements-analyst-open-questions.md`.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class PlayerCrmFieldsType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('skillLevel', TextType::class, ['required' => false])
            ->add('submit', SubmitType::class, ['label' => 'Save changes']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
    }
}
