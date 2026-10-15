<?php

declare(strict_types=1);

namespace App\Scheduling\Form;

use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\Extension\Core\Type\TextareaType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;

/**
 * AC-02-36: "Option B - Decline" — an optional text reason.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class DeclineAssignmentType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('reason', TextareaType::class, ['required' => false])
            ->add('submit', SubmitType::class, ['label' => 'Decline']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
    }
}
