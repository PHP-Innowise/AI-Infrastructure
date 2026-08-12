<?php

declare(strict_types=1);

namespace App\Identity\Form;

use App\Identity\Entity\Gender;
use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\ChoiceType;
use Symfony\Component\Form\Extension\Core\Type\DateType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\Extension\Core\Type\TextType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;
use Symfony\Component\Validator\Constraints\GreaterThan;
use Symfony\Component\Validator\Constraints\LessThanOrEqual;
use Symfony\Component\Validator\Constraints\NotBlank;

/**
 * AC-01-16/21: name, age, gender required; school is optional.
 *
 * US-01.03 § Validation states the range outright — "Age: 1-18 years
 * (children only, adults use own accounts)" — and `ChildProfileService`
 * enforces it as the invariant. It is restated here as field constraints so
 * a parent sees the reason on the date they typed rather than as a
 * form-level error; the previous note that "CURRENT_DATE-relative rules
 * cannot live in a Form constraint cleanly" is simply not true of
 * `GreaterThan`/`LessThanOrEqual`, both of which accept relative dates.
 *
 * The two bounds, exactly:
 *   - born on or before one year ago  => at least 1
 *   - born strictly after 19 years ago => at most 18 (someone born exactly
 *     19 years ago is 19 today, and belongs on their own account)
 *
 * Together they also make a date of birth in the future impossible, which is
 * how a "child" born in 2030 got in: the range check existed, and an
 * unsigned date difference reported that child as 3 years old. See
 * `PlayerProfile::ageInYears()`.
 *
 * @extends AbstractType<array<string, mixed>>
 */
final class ChildProfileType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('firstName', TextType::class, ['constraints' => [new NotBlank()]])
            ->add('dateOfBirth', DateType::class, [
                'widget' => 'single_text',
                'constraints' => [
                    new NotBlank(),
                    new LessThanOrEqual(
                        value: '-1 year',
                        message: 'A child profile is for someone aged 1 to 18. Check the date of birth.',
                    ),
                    new GreaterThan(
                        value: '-19 years',
                        message: 'A child profile is for someone aged 1 to 18 — an adult uses their own account.',
                    ),
                ],
            ])
            ->add('gender', ChoiceType::class, [
                'required' => false,
                'choices' => Gender::choices(),
            ])
            ->add('schoolOrTeam', TextType::class, ['required' => false, 'label' => 'School (optional)'])
            ->add('submit', SubmitType::class, ['label' => 'Save child profile']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
    }
}
