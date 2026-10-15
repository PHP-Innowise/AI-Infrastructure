<?php

declare(strict_types=1);

namespace App\Scheduling\Form;

use App\Identity\Entity\PlayerProfile;
use Symfony\Bridge\Doctrine\Form\Type\EntityType;
use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;

/**
 * AC-02-45/Q-02.09: the trainer manually adds a player to the roster —
 * allowed even over capacity, per Q-02.09's resolved default.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class ManualAddPlayerType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('player', EntityType::class, [
                'class' => PlayerProfile::class,
                'choices' => $options['candidates'],
                'choice_label' => 'firstName',
            ])
            ->add('submit', SubmitType::class, ['label' => 'Add player']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
        $resolver->setRequired(['candidates']);
    }
}
