<?php

declare(strict_types=1);

namespace App\Crm\Form;

use App\Identity\Entity\SkillLevel;
use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\ChoiceType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;

/**
 * AC-03-33: "the trainer can edit the player profile's limited fields
 * (skill level, notes)" — "notes" here is the dedicated Notes section
 * (AddNoteType), not a form field on this one, so `skillLevel` is the only
 * field this Form owns.
 *
 * It offers the same four levels the segmentation filter beside it has always
 * offered (US-03.06). It used to be free text, which meant a trainer could
 * type a value no filter could match and no event restriction could
 * recognise — see `SkillLevel` for what that cost. A choice list is also the
 * only way US-03.09's own walkthrough works at all: "Set Skill Level:
 * Beginner... Sarah now appears in filtered lists for Beginners."
 *
 * @extends AbstractType<array<string, mixed>>
 */
final class PlayerCrmFieldsType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        /** @var string|null $current */
        $current = $options['currentSkillLevel'];

        $builder
            ->add('skillLevel', ChoiceType::class, [
                'required' => false,
                'choices' => $this->choicesIncluding($current),
                'placeholder' => 'Not set',
            ])
            ->add('submit', SubmitType::class, ['label' => 'Save changes']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver
            ->setDefaults(['csrf_protection' => true, 'currentSkillLevel' => null])
            ->setAllowedTypes('currentSkillLevel', ['null', 'string']);
    }

    /**
     * A profile written before this field was a choice list can hold a word
     * that is not one of the four. That value stays offered, once, so opening
     * the form shows what is actually stored instead of an innocent-looking
     * "Not set" the trainer never chose — they can keep it or replace it, but
     * the screen does not misreport the database.
     *
     * @return array<string, string>
     */
    private function choicesIncluding(?string $current): array
    {
        $choices = SkillLevel::choices();

        if (null !== $current && '' !== $current && null === SkillLevel::canonicalize($current)) {
            $choices[$current] = $current;
        }

        return $choices;
    }
}
