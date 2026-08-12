<?php

declare(strict_types=1);

namespace App\Administration\Form;

use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\ChoiceType;
use Symfony\Component\Form\Extension\Core\Type\EmailType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\Extension\Core\Type\TelType;
use Symfony\Component\Form\Extension\Core\Type\TextType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;
use Symfony\Component\Validator\Constraints\Email;
use Symfony\Component\Validator\Constraints\NotBlank;

/**
 * AC-01-71, AC-07-13..15: Super Admin edits any account. Role is
 * deliberately never a field here — it is not on this form at all, which is
 * what keeps it view-only; deactivation/reactivation/deletion are their own
 * dedicated, confirmation-gated actions, not a status dropdown on this form
 * (AC-01-52/53's own confirm-and-audit flow, which a bare checkbox here
 * would bypass — status stays reachable from the same user row instead,
 * `administration/user_show.html.twig`'s own Deactivate/Reactivate links).
 *
 * AC-07-14's "player-specific profile details (age, gender, etc.) are
 * editable when the user is a player" — `isPlayer` gates two extra fields
 * on, gender and school/team. Age (date of birth) is deliberately excluded:
 * AC-01-48 (already built, already covered by Epic-01's own 78/78) states
 * date of birth stays read-only for every editor of a player profile
 * everywhere else in this codebase (`PlayerProfile::updateProfile()` itself
 * has no date-of-birth parameter at all) — the epic that actually owns this
 * field's mutability wins over AC-07-14's looser "age... etc." wording,
 * matching this task's own precedent for resolving an epic-vs-epic
 * disagreement (A2, User Role Editor). Recorded in the coder's final
 * report.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class AdminEditAccountType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('firstName', TextType::class, ['constraints' => [new NotBlank()]])
            ->add('lastName', TextType::class, ['constraints' => [new NotBlank()]])
            ->add('email', EmailType::class, [
                'constraints' => [new NotBlank(), new Email()],
                'help' => 'Changing the email does not require re-verification in this MVP build.',
            ])
            ->add('phone', TelType::class, ['required' => false]);

        if (true === $options['isPlayer']) {
            $builder
                ->add('playerGender', ChoiceType::class, [
                    'required' => false,
                    'label' => 'Gender',
                    'choices' => ['Female' => 'female', 'Male' => 'male', 'Prefer not to say' => 'unspecified'],
                ])
                ->add('playerSchoolOrTeam', TextType::class, ['required' => false, 'label' => 'School / Team']);
        }

        $builder->add('submit', SubmitType::class, ['label' => 'Save account']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true, 'isPlayer' => false]);
        $resolver->setAllowedTypes('isPlayer', 'bool');
    }
}
