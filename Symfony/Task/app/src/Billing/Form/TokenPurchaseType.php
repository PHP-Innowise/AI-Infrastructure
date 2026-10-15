<?php

declare(strict_types=1);

namespace App\Billing\Form;

use App\Billing\Entity\TokenPackage;
use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\ChoiceType;
use Symfony\Component\Form\Extension\Core\Type\IntegerType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;
use Symfony\Component\Validator\Constraints\GreaterThan;

/**
 * AC-05-4: "select a package... or a custom amount." Exactly one of the
 * two is expected to be filled in; the controller decides which the
 * submission actually used (a chosen package takes precedence over a
 * stray custom amount left over from a prior, corrected input).
 *
 * `choices` is a plain id-keyed `ChoiceType`, not `EntityType`: the
 * controller already loaded the trainer's active packages to render the
 * page (AC-05-4's own "see the current balance... select a package"
 * screen), so re-querying them again inside the form type would be
 * redundant.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class TokenPurchaseType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        /** @var list<TokenPackage> $packages */
        $packages = $options['packages'];

        $choices = [];
        foreach ($packages as $package) {
            $choices[$package->getLabel()] = $package->getId();
        }

        $builder
            ->add('packageId', ChoiceType::class, ['choices' => $choices, 'required' => false, 'placeholder' => 'Choose a package'])
            ->add('customTokenCount', IntegerType::class, ['required' => false, 'constraints' => [new GreaterThan(0)]])
            ->add('submit', SubmitType::class, ['label' => 'Purchase']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
        $resolver->setRequired(['packages']);
    }
}
