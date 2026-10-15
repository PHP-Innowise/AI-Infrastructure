<?php

declare(strict_types=1);

namespace App\Forms\Form;

use App\Identity\Entity\Gender;
use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\CheckboxType;
use Symfony\Component\Form\Extension\Core\Type\ChoiceType;
use Symfony\Component\Form\Extension\Core\Type\DateType;
use Symfony\Component\Form\Extension\Core\Type\PasswordType;
use Symfony\Component\Form\Extension\Core\Type\RepeatedType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;
use Symfony\Component\Validator\Constraints\IsTrue;
use Symfony\Component\Validator\Constraints\Length;
use Symfony\Component\Validator\Constraints\NotBlank;

/**
 * AC-08-23/24: name/email are pre-filled read-only from the submission
 * (rendered directly by the template, never bound as form fields here — the
 * account's email and the player's name come from `FormSubmission` itself,
 * never from user input on this screen, exactly as the route table states).
 *
 * **`dateOfBirth` is a deliberate addition beyond api-designer-spec.md's own
 * literal field list** ("password, terms acceptance; name/email pre-filled
 * read-only"). `PlayerProfile.date_of_birth` is `NOT NULL`
 * (database-designer-schema.md, settled by Epic-01) and Epic-08's own
 * closed field vocabulary (BR-08-2: Text/Email/Dropdown/Multi-select only)
 * has no date type, so a trainer-authored camp form cannot reliably supply
 * a structured date of birth — parsing one out of a freeform "age" text
 * answer would be guesswork, not a fact. Asking for it here, once, is the
 * only honest way to satisfy `Identity`'s own invariant (A1's child/adult
 * branch also depends on it — see
 * `PlayerRegistrationService::registerViaCampConversion()`) without
 * fabricating a value. Recorded in the coder's final report.
 *
 * @see specs/api-designer-spec.md "Forms module" — `ConvertSubmissionToAccountType`
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class ConvertSubmissionToAccountType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('plainPassword', RepeatedType::class, [
                'type' => PasswordType::class,
                'first_options' => ['label' => 'Password'],
                'second_options' => ['label' => 'Confirm password'],
                'invalid_message' => 'The password fields must match.',
                'constraints' => [new NotBlank(), new Length(min: 8)],
            ])
            ->add('dateOfBirth', DateType::class, [
                'label' => 'Date of birth',
                'widget' => 'single_text',
                'constraints' => [new NotBlank()],
            ])
            ->add('gender', ChoiceType::class, [
                'label' => 'Gender',
                'required' => false,
                'choices' => Gender::choices(),
            ])
            ->add('acceptTerms', CheckboxType::class, [
                'label' => 'I accept the terms of service',
                'mapped' => false,
                'constraints' => [new IsTrue(message: 'You must accept the terms of service to continue.')],
            ])
            ->add('submit', SubmitType::class, ['label' => 'Create Account']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
    }
}
