<?php

declare(strict_types=1);

namespace App\Forms\Service;

use App\Forms\Dto\FormField;
use App\Forms\Entity\Form;
use Symfony\Component\Form\Extension\Core\Type\ChoiceType;
use Symfony\Component\Form\Extension\Core\Type\EmailType;
use Symfony\Component\Form\Extension\Core\Type\FormType;
use Symfony\Component\Form\Extension\Core\Type\TextType;
use Symfony\Component\Form\FormFactoryInterface;
use Symfony\Component\Form\FormInterface;
use Symfony\Component\Form\FormTypeInterface;
use Symfony\Component\Validator\Constraints\Email;
use Symfony\Component\Validator\Constraints\NotBlank;

/**
 * AC-08-14: "A dynamically built Symfony Form, assembled at runtime from
 * `Form.fieldDefinitions`... via `FormFactoryInterface` — not a static
 * class" (api-designer-spec.md's own words, verbatim) — the field list is
 * per-`Form` data, so no fixed `AbstractType` could express it.
 *
 * **The honeypot** (BR-08 "Risks & Mitigations — Form Spam Submissions"):
 * `HONEYPOT_FIELD` — `mapped => false` so it never lands in the answer map
 * at all, present in every dynamically built form regardless of the
 * trainer's own field list. A human never sees it (the template hides it
 * with CSS, never `type="hidden"`, which unsophisticated bots specifically
 * skip); a bot's generic autofill fills it. `PublicFormController` checks
 * it and, if filled, silently pretends success without ever calling
 * `FormSubmissionService` — "never surfaced as a validation error, to avoid
 * tipping off the bot" (api-designer-spec.md).
 */
final readonly class PublicSubmissionFormBuilder
{
    public const HONEYPOT_FIELD = 'website';

    public function __construct(
        private FormFactoryInterface $formFactory,
    ) {
    }

    /**
     * @return FormInterface<array<string, mixed>>
     */
    public function build(Form $form): FormInterface
    {
        $builder = $this->formFactory->createNamedBuilder('form_submission', FormType::class, null, ['csrf_protection' => true]);

        foreach ($form->getFields() as $field) {
            $builder->add($field->id, $this->widgetClassFor($field), $this->optionsFor($field));
        }

        $builder->add(self::HONEYPOT_FIELD, TextType::class, ['required' => false, 'mapped' => false]);

        return $builder->getForm();
    }

    /**
     * @return class-string<FormTypeInterface<mixed>>
     */
    private function widgetClassFor(FormField $field): string
    {
        return match ($field->type) {
            FormField::TYPE_EMAIL => EmailType::class,
            FormField::TYPE_DROPDOWN, FormField::TYPE_MULTISELECT => ChoiceType::class,
            default => TextType::class,
        };
    }

    /**
     * @return array<string, mixed>
     */
    private function optionsFor(FormField $field): array
    {
        $constraints = $field->required ? [new NotBlank()] : [];

        if (FormField::TYPE_EMAIL === $field->type) {
            // BR-08-9: "the email field must be a valid format."
            $constraints[] = new Email();
        }

        $options = [
            'label' => $field->label,
            'required' => $field->required,
            'constraints' => $constraints,
        ];

        if ($field->isChoice()) {
            $choices = $field->options ?? [];
            $options['choices'] = array_combine($choices, $choices);
            $options['multiple'] = FormField::TYPE_MULTISELECT === $field->type;
            $options['placeholder'] = FormField::TYPE_DROPDOWN === $field->type ? 'Select…' : null;
        }

        return $options;
    }
}
