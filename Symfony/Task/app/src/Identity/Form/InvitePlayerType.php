<?php

declare(strict_types=1);

namespace App\Identity\Form;

use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\EmailType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;

/**
 * AC-03-50/52: a coach's player invite — unlike `InviteCoachType`, the
 * recipient email is optional (the coach may just copy the link instead of
 * emailing it), matching `specs/api-designer-spec.md:421`'s
 * `InvitePlayerType (optional recipient email)`.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class InvitePlayerType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('email', EmailType::class, ['required' => false])
            ->add('submit', SubmitType::class, ['label' => 'Generate invite link']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
    }
}
