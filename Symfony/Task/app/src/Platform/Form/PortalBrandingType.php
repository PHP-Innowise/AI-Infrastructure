<?php

declare(strict_types=1);

namespace App\Platform\Form;

use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\ColorType;
use Symfony\Component\Form\Extension\Core\Type\FileType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;
use Symfony\Component\Validator\Constraints\File;

/**
 * AC-01-60/61: logo upload (PNG/JPG/SVG, max 2MB) and a primary colour.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class PortalBrandingType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('logo', FileType::class, [
                'required' => false,
                'mapped' => false,
                'constraints' => [new File(maxSize: '2M', mimeTypes: ['image/png', 'image/jpeg', 'image/svg+xml'])],
            ])
            ->add('primaryColorHex', ColorType::class, ['required' => false, 'label' => 'Primary brand colour'])
            ->add('submit', SubmitType::class, ['label' => 'Save branding']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
    }
}
