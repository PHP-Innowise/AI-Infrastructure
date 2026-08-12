<?php

declare(strict_types=1);

namespace App\Identity\Form;

use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\ChoiceType;
use Symfony\Component\Form\Extension\Core\Type\DateType;
use Symfony\Component\Form\Extension\Core\Type\EmailType;
use Symfony\Component\Form\Extension\Core\Type\PasswordType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\Extension\Core\Type\TelType;
use Symfony\Component\Form\Extension\Core\Type\TextType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;
use Symfony\Component\Validator\Constraints\Email;
use Symfony\Component\Validator\Constraints\Length;
use Symfony\Component\Validator\Constraints\LessThanOrEqual;
use Symfony\Component\Validator\Constraints\NotBlank;
use Symfony\Component\Validator\Constraints\Regex;

/**
 * AC-01-10: name, email, password, parent phone, and player name/age/gender.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class PlayerRegistrationType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('accountFirstName', TextType::class, ['label' => 'Your first name', 'constraints' => [new NotBlank()]])
            ->add('accountLastName', TextType::class, ['label' => 'Your last name', 'constraints' => [new NotBlank()]])
            ->add('email', EmailType::class, ['constraints' => [new NotBlank(), new Email()]])
            // Labelled explicitly: without this Symfony derives the label from
            // the property and a first-time visitor is asked for a
            // "Plain password" on the very first screen they ever see. The
            // minimum is stated up front rather than only after a failed
            // submission, since the constraint below already enforces it.
            ->add('plainPassword', PasswordType::class, [
                'label' => 'Password',
                'help' => 'At least 8 characters.',
                'constraints' => [new NotBlank(), new Length(min: 8)],
            ])
            ->add('parentPhone', TelType::class, [
                'required' => false,
                'label' => 'Phone',
                'constraints' => [new Regex(pattern: '/^[0-9()+\-.\s]{7,32}$/', message: 'Enter a valid phone number.')],
            ])
            ->add('playerFirstName', TextType::class, ['label' => 'Player first name', 'constraints' => [new NotBlank()]])
            // No 1-18 range here, deliberately: US-01.03 scopes that to child
            // profiles ("adults use own accounts") and the same section notes
            // that "the parent account is treated as a player account (parent
            // can train themselves)" — so the player being registered may be
            // any age. A date of birth in the future is a different matter and
            // is nobody's age.
            ->add('playerDateOfBirth', DateType::class, [
                'label' => 'Player date of birth',
                'widget' => 'single_text',
                'constraints' => [
                    new NotBlank(),
                    new LessThanOrEqual(value: 'today', message: 'A date of birth cannot be in the future.'),
                ],
            ])
            ->add('playerGender', ChoiceType::class, [
                'required' => false,
                'choices' => ['Female' => 'female', 'Male' => 'male', 'Prefer not to say' => 'unspecified'],
            ])
            ->add('submit', SubmitType::class, ['label' => 'Register']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
    }
}
