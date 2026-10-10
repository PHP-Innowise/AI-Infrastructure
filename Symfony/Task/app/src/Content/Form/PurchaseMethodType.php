<?php

declare(strict_types=1);

namespace App\Content\Form;

use App\Content\Service\PurchasePlaylistAccessService;
use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\ChoiceType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\Extension\Core\Type\TextType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;

/**
 * AC-04-22, BR-04-6: the locked-content checkout form — `method`: card
 * (`usd`) or token, plus an optional coupon code field per
 * `specs/api-designer-spec.md`'s `PurchaseMethodType` naming (coupon
 * validation itself is Growth/Epic-06 — out of this epic's reach; the field
 * is accepted and simply ignored today).
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class PurchaseMethodType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('method', ChoiceType::class, [
                'choices' => ['Tokens' => PurchasePlaylistAccessService::METHOD_TOKEN, 'Card' => PurchasePlaylistAccessService::METHOD_USD],
            ])
            ->add('couponCode', TextType::class, ['required' => false])
            ->add('submit', SubmitType::class, ['label' => 'Purchase']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
    }
}
