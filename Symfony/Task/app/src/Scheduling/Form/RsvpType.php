<?php

declare(strict_types=1);

namespace App\Scheduling\Form;

use App\Scheduling\Entity\Event;
use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\ChoiceType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;

/**
 * AC-02-23/60: "Pay $25 OR 2 tokens" — the choices offered are exactly
 * $options['event']->availablePaymentMethods(), never a client-supplied
 * price (specs/api-designer-spec.md:507, "never a client-supplied price").
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class RsvpType extends AbstractType
{
    private const LABELS = [
        Event::PAYMENT_FREE => 'Free',
        Event::PAYMENT_USD => 'Pay with card',
        Event::PAYMENT_TOKEN => 'Pay with tokens',
    ];

    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        /** @var Event $event */
        $event = $options['event'];
        $available = $event->availablePaymentMethods();

        $choices = [];
        foreach ($available as $method) {
            $choices[self::LABELS[$method]] = $method;
        }

        $builder
            ->add('paymentMethod', ChoiceType::class, [
                'choices' => $choices,
                'expanded' => true,
                'data' => $available[0],
            ])
            ->add('submit', SubmitType::class, ['label' => Event::PAYMENT_FREE === $available[0] && 1 === \count($available) ? 'RSVP' : 'Register & Pay']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
        $resolver->setRequired(['event']);
    }
}
