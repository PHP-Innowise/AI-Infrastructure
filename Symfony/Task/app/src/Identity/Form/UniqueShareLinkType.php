<?php

declare(strict_types=1);

namespace App\Identity\Form;

use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\EmailType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\Extension\Core\Type\TextType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;
use Symfony\Component\Validator\Constraints\Email;

/**
 * AC-03-61 (optional MVP): a trainer generates a unique, one-time link for
 * one named player/parent — recipient name/email, matching
 * `specs/api-designer-spec.md:420`'s `UniqueShareLinkType (recipient
 * name/email)`.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class UniqueShareLinkType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('recipientName', TextType::class, ['required' => false])
            ->add('email', EmailType::class, ['required' => false, 'constraints' => [new Email()]])
            ->add('submit', SubmitType::class, ['label' => 'Generate unique link']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
    }
}
