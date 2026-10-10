<?php

declare(strict_types=1);

namespace App\Identity\Form;

use App\Identity\Entity\AccountRole;
use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\CheckboxType;
use Symfony\Component\Form\Extension\Core\Type\FileType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\Extension\Core\Type\TelType;
use Symfony\Component\Form\Extension\Core\Type\TextareaType;
use Symfony\Component\Form\Extension\Core\Type\TextType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;
use Symfony\Component\Validator\Constraints\File;
use Symfony\Component\Validator\Constraints\NotBlank;
use Symfony\Component\Validator\Constraints\Regex;

/**
 * AC-01-48..51: common fields for every role, plus each role's own extra
 * fields resolved server-side from the actor's role — never client-supplied.
 * Email, role and (for players) skill level are never fields on this form at
 * all, which is what keeps them read-only (AC-01-48).
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class EditProfileType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('firstName', TextType::class, ['constraints' => [new NotBlank()]])
            ->add('lastName', TextType::class, ['constraints' => [new NotBlank()]])
            ->add('phone', TelType::class, [
                'required' => false,
                'constraints' => [new Regex(pattern: '/^[0-9()+\-.\s]{7,32}$/', message: 'Enter a valid phone number.')],
            ])
            ->add('schoolOrOrganization', TextType::class, ['required' => false])
            ->add('photo', FileType::class, [
                'required' => false,
                'mapped' => false,
                'constraints' => [new File(maxSize: '2M', mimeTypes: ['image/png', 'image/jpeg', 'image/svg+xml'])],
            ]);

        /** @var AccountRole $role */
        $role = $options['role'];

        if (AccountRole::Player === $role) {
            $builder
                ->add('schoolOrTeam', TextType::class, ['required' => false])
                ->add('jerseyNumber', TextType::class, ['required' => false]);
        }

        if (AccountRole::Coach === $role) {
            $builder
                ->add('bio', TextareaType::class, ['required' => false])
                ->add('credentials', TextareaType::class, ['required' => false])
                ->add('certifications', TextareaType::class, ['required' => false])
                ->add('isPublicProfile', CheckboxType::class, ['required' => false, 'label' => 'Make my profile public']);
        }

        if (AccountRole::Trainer === $role) {
            $builder
                ->add('organizationAddress', TextType::class, ['required' => false])
                ->add('organizationWebsite', TextType::class, ['required' => false])
                ->add('organizationDescription', TextareaType::class, ['required' => false]);
        }

        $builder->add('submit', SubmitType::class, ['label' => 'Save changes']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
        $resolver->setRequired('role');
        $resolver->setAllowedTypes('role', AccountRole::class);
    }
}
