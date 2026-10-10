<?php

declare(strict_types=1);

namespace App\Forms\Dto;

/**
 * One entry in `Form::$fieldDefinitions` (the JSONB array the schema doc
 * names — "field_definitions as JSONB, not a child table": field types are a
 * closed four-value set, BR-08-2, and no query anywhere needs to find fields
 * individually). Not a Doctrine entity or embeddable — a plain value object
 * the entity (de)serializes through at its own boundary.
 *
 * **Two reserved, non-removable ids.** The epic gives every field an
 * arbitrary trainer-authored id, but something has to anchor "the name
 * field" and "the email field" deterministically — AC-08-2's "minimum of 1
 * field required (participant name)", the BR-08-9 email-format rule, and
 * `form_submission.contact_email` (denormalized for the BR-08-10 uniqueness
 * check) all need one. Every template seeds `FIELD_PARTICIPANT_NAME` (text)
 * and `FIELD_PARTICIPANT_EMAIL` (email); `Form` refuses to persist a field
 * list missing either. This is this implementation's own resolution of a gap
 * the epic leaves implicit, not a stated requirement — recorded in the
 * coder's final report.
 *
 * @see specs/database-designer-schema.md "Form Field Definition"
 */
final readonly class FormField
{
    public const TYPE_TEXT = 'text';
    public const TYPE_EMAIL = 'email';
    public const TYPE_DROPDOWN = 'dropdown';
    public const TYPE_MULTISELECT = 'multiselect';

    /** @var list<string> */
    public const TYPES = [self::TYPE_TEXT, self::TYPE_EMAIL, self::TYPE_DROPDOWN, self::TYPE_MULTISELECT];

    public const FIELD_PARTICIPANT_NAME = 'participant_name';
    public const FIELD_PARTICIPANT_EMAIL = 'participant_email';

    /**
     * @param ?list<string> $options required (non-empty, no blank entries)
     *                               for dropdown/multiselect, must be null for text/email
     */
    public function __construct(
        public string $id,
        public string $type,
        public string $label,
        public bool $required,
        public ?array $options = null,
    ) {
        if ('' === trim($id)) {
            throw new \InvalidArgumentException('A form field requires a non-empty id.');
        }

        if (!\in_array($type, self::TYPES, true)) {
            throw new \InvalidArgumentException(sprintf('Unknown form field type "%s".', $type));
        }

        if ('' === trim($label)) {
            throw new \InvalidArgumentException('A form field requires a non-empty label.');
        }

        $needsOptions = self::TYPE_DROPDOWN === $type || self::TYPE_MULTISELECT === $type;

        if ($needsOptions) {
            $blankOption = null !== $options && [] !== array_filter($options, static fn (string $option): bool => '' === trim($option));

            if (null === $options || [] === $options || $blankOption) {
                throw new \InvalidArgumentException(sprintf('A "%s" field requires at least one non-empty option.', $type));
            }
        } elseif (null !== $options) {
            throw new \InvalidArgumentException(sprintf('A "%s" field must not carry options.', $type));
        }
    }

    /**
     * @param array<string, mixed> $data
     */
    public static function fromArray(array $data): self
    {
        $rawOptions = $data['options'] ?? null;

        return new self(
            \is_string($data['id'] ?? null) ? $data['id'] : '',
            \is_string($data['type'] ?? null) ? $data['type'] : '',
            \is_string($data['label'] ?? null) ? $data['label'] : '',
            (bool) ($data['required'] ?? false),
            \is_array($rawOptions) ? array_values(array_map(static fn (mixed $option): string => (string) $option, $rawOptions)) : null,
        );
    }

    /**
     * @return array{id: string, type: string, label: string, required: bool, options: ?list<string>}
     */
    public function toArray(): array
    {
        return [
            'id' => $this->id,
            'type' => $this->type,
            'label' => $this->label,
            'required' => $this->required,
            'options' => $this->options,
        ];
    }

    public function isChoice(): bool
    {
        return self::TYPE_DROPDOWN === $this->type || self::TYPE_MULTISELECT === $this->type;
    }

    public function isReserved(): bool
    {
        return self::FIELD_PARTICIPANT_NAME === $this->id || self::FIELD_PARTICIPANT_EMAIL === $this->id;
    }
}
