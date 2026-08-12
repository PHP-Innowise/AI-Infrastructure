<?php

declare(strict_types=1);

namespace App\Crm\Form;

use App\Crm\Entity\Label;
use Symfony\Bridge\Doctrine\Form\Type\EntityType;
use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;

/**
 * AC-03-12: multi-select from every label this trainer has created.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class ApplyLabelsType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('labels', EntityType::class, [
                'class' => Label::class,
                'choices' => $options['availableLabels'],
                'choice_label' => 'name',
                'multiple' => true,
                'expanded' => true,
            ])
            ->add('submit', SubmitType::class, ['label' => 'Apply labels']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
        $resolver->setRequired(['availableLabels']);
    }
}
