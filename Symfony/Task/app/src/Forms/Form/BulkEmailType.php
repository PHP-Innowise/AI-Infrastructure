<?php

declare(strict_types=1);

namespace App\Forms\Form;

use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\Extension\Core\Type\TextareaType;
use Symfony\Component\Form\Extension\Core\Type\TextType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;
use Symfony\Component\Validator\Constraints\NotBlank;

/**
 * AC-08-21: subject + body, sent to every one of a camp's participants.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class BulkEmailType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('subject', TextType::class, ['constraints' => [new NotBlank()]])
            ->add('body', TextareaType::class, ['constraints' => [new NotBlank()]])
            ->add('submit', SubmitType::class, ['label' => 'Send Email']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
    }
}
