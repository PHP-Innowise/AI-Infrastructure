<?php

declare(strict_types=1);

namespace App\Scheduling\Form;

use App\Identity\Entity\CoachMembership;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Entity\Gender;
use App\Identity\Entity\SkillLevel;
use App\Scheduling\Entity\Event;
use Symfony\Bridge\Doctrine\Form\Type\EntityType;
use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\CheckboxType;
use Symfony\Component\Form\Extension\Core\Type\ChoiceType;
use Symfony\Component\Form\Extension\Core\Type\DateTimeType;
use Symfony\Component\Form\Extension\Core\Type\IntegerType;
use Symfony\Component\Form\Extension\Core\Type\NumberType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\Extension\Core\Type\TextareaType;
use Symfony\Component\Form\Extension\Core\Type\TextType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\Form\FormError;
use Symfony\Component\Form\FormEvent;
use Symfony\Component\Form\FormEvents;
use Symfony\Component\OptionsResolver\OptionsResolver;
use Symfony\Component\Validator\Constraints\Length;
use Symfony\Component\Validator\Constraints\NotBlank;
use Symfony\Component\Validator\Constraints\Range;

/**
 * AC-02-1/3, BR-02-1..4: Event Builder's create/edit/duplicate form — one
 * shape for all three (specs/api-designer-spec.md abbreviates its ~dozen
 * fields identically for create and edit).
 *
 * `skillLevels` was comma-separated free text on the grounds that no epic
 * named a vocabulary. One does: Epic-03's own segmentation filter (US-03.06)
 * fixes the four levels, and eligibility compares this field against a
 * player's profile value exactly — so free text here meant a trainer could
 * restrict an event to "intermediate" and exclude every player whose profile
 * said "Intermediate". It is now the same closed list the profile offers.
 *
 * `genders` had the same mismatch and was fixed the same way once it had
 * been looked at on its own: a player's gender comes from a fixed
 * female/male/unspecified choice, so a restriction typed as "Female" matched
 * nobody at all. Both axes are now closed lists over the vocabulary the
 * profile itself uses.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class EventType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        /** @var \DateTimeZone $timezone */
        $timezone = $options['timezone'];
        $timezoneName = $timezone->getName();

        $builder
            ->add('title', TextType::class, ['constraints' => [new NotBlank(), new Length(max: 100)]])
            ->add('eventType', ChoiceType::class, ['choices' => array_combine(
                array_map(static fn (string $t): string => ucwords(str_replace('_', ' ', $t)), Event::types()),
                Event::types(),
            )])
            ->add('startsAt', DateTimeType::class, [
                'widget' => 'single_text',
                'input' => 'datetime_immutable',
                'model_timezone' => $timezoneName,
                'view_timezone' => $timezoneName,
                'constraints' => [new NotBlank()],
            ])
            ->add('endsAt', DateTimeType::class, [
                'widget' => 'single_text',
                'input' => 'datetime_immutable',
                'model_timezone' => $timezoneName,
                'view_timezone' => $timezoneName,
                'constraints' => [new NotBlank()],
            ])
            ->add('location', TextType::class, ['constraints' => [new NotBlank()]])
            ->add('capacity', IntegerType::class, ['constraints' => [new Range(min: 1, max: 999)]])
            ->add('visibility', ChoiceType::class, [
                'choices' => ['Public' => Event::VISIBILITY_PUBLIC, 'Private / Invite Only' => Event::VISIBILITY_PRIVATE],
            ])
            ->add('description', TextareaType::class, ['required' => false])
            ->add('minAge', IntegerType::class, ['required' => false, 'constraints' => [new Range(min: 0, max: 120)]])
            ->add('maxAge', IntegerType::class, ['required' => false, 'constraints' => [new Range(min: 0, max: 120)]])
            // BR-02-5's eligibility axis, chosen from the same four levels a
            // player's profile can hold. Typed free-hand it silently excluded
            // players whose profile said the same word with different
            // capitalisation — see SkillLevel.
            ->add('skillLevels', ChoiceType::class, [
                'required' => false,
                'multiple' => true,
                'expanded' => true,
                'choices' => SkillLevel::choices(),
                'label' => 'Skill levels',
                'help' => 'Leave all unchecked to open the event to every skill level.',
            ])
            // BR-02-5's other eligibility axis, and the same story as
            // skillLevels above: typed free-hand, it was compared exactly
            // against a value the player never types — their profile stores
            // `female`, so an event restricted to "Female" matched nobody.
            ->add('genders', ChoiceType::class, [
                'required' => false,
                'multiple' => true,
                'expanded' => true,
                'choices' => Gender::choices(),
                'label' => 'Genders',
                'help' => 'Leave all unchecked to open the event to everyone.',
            ])
            ->add('usdPricingEnabled', CheckboxType::class, ['required' => false])
            ->add('usdPrice', NumberType::class, ['required' => false, 'scale' => 2, 'html5' => true])
            ->add('tokenPricingEnabled', CheckboxType::class, ['required' => false])
            ->add('tokenPrice', IntegerType::class, ['required' => false, 'constraints' => [new Range(min: 1)]])
            ->add('invitedPlayers', EntityType::class, [
                'class' => PlayerProfile::class,
                'choices' => $options['invitablePlayers'],
                'choice_label' => 'firstName',
                'multiple' => true,
                'required' => false,
            ])
            ->add('coach', EntityType::class, [
                'class' => CoachMembership::class,
                'choices' => $options['assignableCoaches'],
                'choice_label' => static fn (CoachMembership $c): string => $c->getAccount()->getProfile()?->getFullName() ?? $c->getAccount()->getEmail(),
                'required' => false,
                'placeholder' => 'No coach assigned',
            ])
            ->add('coachOverrideReason', TextType::class, [
                'required' => false,
                'help' => 'Required only if a scheduling-conflict warning is shown.',
            ])
            ->add('submit', SubmitType::class, ['label' => (string) $options['submitLabel']]);

        // BR-02-2/AC-02-3: start not in the past, end after start; BR-02-3:
        // a paid event needs a positive amount. Event::guardDates()/
        // setUsdPricing() enforce these same rules structurally on the
        // entity, but an uncaught InvalidArgumentException from there would
        // surface as a bare 500 — the entity has no HTTP concept to map
        // itself to (architect-architecture.md's layering: entities "do not
        // know HTTP... controllers"). Checking here first turns every one
        // of these into a normal, visible form error instead, matching how
        // "AC-01-71/BR-01-2" duplicate-email is surfaced as a form error
        // rather than a raw constraint violation elsewhere in this codebase.
        $builder->addEventListener(FormEvents::POST_SUBMIT, function (FormEvent $event) use ($options): void {
            $form = $event->getForm();
            $data = $form->getData();

            if (!\is_array($data)) {
                return;
            }

            $startsAt = $data['startsAt'] ?? null;
            $endsAt = $data['endsAt'] ?? null;

            if ($startsAt instanceof \DateTimeImmutable && true !== ($options['skipPastCheck'] ?? false) && $startsAt < new \DateTimeImmutable()) {
                $form->get('startsAt')->addError(new FormError('The start date/time cannot be in the past.'));
            }

            if ($startsAt instanceof \DateTimeImmutable && $endsAt instanceof \DateTimeImmutable && $endsAt <= $startsAt) {
                $form->get('endsAt')->addError(new FormError('The end date/time must be after the start date/time.'));
            }

            if (true === ($data['usdPricingEnabled'] ?? false) && (float) ($data['usdPrice'] ?? 0) <= 0.0) {
                $form->get('usdPrice')->addError(new FormError('A paid event requires an amount greater than zero.'));
            }
        });
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults([
            'csrf_protection' => true,
            'submitLabel' => 'Save event',
            'skipPastCheck' => false,
        ]);
        $resolver->setRequired(['timezone', 'invitablePlayers', 'assignableCoaches']);
    }
}
