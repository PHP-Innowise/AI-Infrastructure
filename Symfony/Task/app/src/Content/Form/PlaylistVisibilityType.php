<?php

declare(strict_types=1);

namespace App\Content\Form;

use App\Content\Entity\Playlist;
use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\ChoiceType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\Form\FormError;
use Symfony\Component\Form\FormEvent;
use Symfony\Component\Form\FormEvents;
use Symfony\Component\OptionsResolver\OptionsResolver;

/**
 * AC-04-17/18/41, A9: the publication/audience toggle — two orthogonal
 * fields covering A9's three valid states in one form, per
 * architect-architecture.md "Playlist visibility storage" and
 * `specs/api-designer-spec.md`'s own `PlaylistVisibilityType` naming.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class PlaylistVisibilityType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('publication', ChoiceType::class, [
                'choices' => ['Public' => true, 'Private' => false],
            ])
            ->add('audience', ChoiceType::class, [
                'choices' => ['Players & Coaches' => Playlist::AUDIENCE_PLAYERS_AND_COACHES, 'Coaches Only' => Playlist::AUDIENCE_COACHES_ONLY],
            ])
            ->add('submit', SubmitType::class, ['label' => 'Save visibility']);

        // A9/AC-04-41: public + coaches-only is never a valid combination.
        // Playlist::setVisibility() enforces this too, but an uncaught
        // \InvalidArgumentException from there would surface as a bare 500
        // — checking here turns it into an ordinary form error instead,
        // matching every other POST_SUBMIT guard in this module.
        $builder->addEventListener(FormEvents::POST_SUBMIT, function (FormEvent $event): void {
            $form = $event->getForm();
            $data = $form->getData();

            if (\is_array($data) && true === ($data['publication'] ?? false) && Playlist::AUDIENCE_COACHES_ONLY === ($data['audience'] ?? null)) {
                $form->get('audience')->addError(new FormError('Public content cannot be coaches-only.'));
            }
        });
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
    }
}
