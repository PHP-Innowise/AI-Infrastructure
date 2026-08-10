<?php

declare(strict_types=1);

namespace App\Administration\Form;

use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\EmailType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\Extension\Core\Type\TelType;
use Symfony\Component\Form\Extension\Core\Type\TextType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;
use Symfony\Component\Validator\Constraints\NotBlank;

/**
 * AC-01-71, AC-07-13..15: Super Admin edits any account. Role is
 * deliberately never a field here — it is not on this form at all, which is
 * what keeps it view-only; deactivation/reactivation/deletion are their own
 * dedicated, confirmation-gated actions, not a status dropdown on this form.
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
                'constraints' => [new NotBlank()],
                'help' => 'Changing the email does not require re-verification in this MVP build.',
            ])
            ->add('phone', TelType::class, ['required' => false])
            ->add('submit', SubmitType::class, ['label' => 'Save account']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
    }
}
