<?php

declare(strict_types=1);

namespace App\Forms\Service;

use App\Forms\Entity\FormSubmission;
use Symfony\Component\DependencyInjection\Attribute\Autowire;

/**
 * The "opaque token, not the raw id" `forms_public_confirmation` and the
 * checkout waypoint routes carry (api-designer-spec.md: "avoids enumerable
 * submission ids appearing in a bookmarkable URL"). A real requirement, not
 * cosmetic: `form_submission` rows are trainer-scoped but not otherwise
 * ownership-checked per row, so a raw sequential id would let one registrant
 * enumerate every OTHER registrant's name/email/age/emergency-contact for
 * the same camp once the tenant is resolved.
 *
 * Stateless (HMAC over the id, keyed by `kernel.secret`) rather than a
 * stored column: no schema deviation, no new table, verified by
 * recomputation. Not a capability token — resolution is not authorization
 * (`FormSubmissionVoter` still governs); this only keeps a bookmarked URL
 * from being a directory of every submission id in sequence.
 */
final readonly class SubmissionTokenFactory
{
    public function __construct(
        #[Autowire('%kernel.secret%')]
        private string $secret,
    ) {
    }

    public function tokenFor(FormSubmission $submission): string
    {
        $id = $submission->getId();
        \assert(null !== $id);

        return $id.'.'.$this->signatureFor($id);
    }

    /**
     * Returns the submission id the token names, or null if the token is
     * malformed or its signature does not verify — never throws, so a
     * forged/guessed token is indistinguishable from any other 404.
     */
    public function decode(string $token): ?int
    {
        $parts = explode('.', $token, 2);

        if (2 !== \count($parts) || !ctype_digit($parts[0])) {
            return null;
        }

        $id = (int) $parts[0];

        return hash_equals($this->signatureFor($id), $parts[1]) ? $id : null;
    }

    private function signatureFor(int $id): string
    {
        return substr(hash_hmac('sha256', (string) $id, $this->secret), 0, 32);
    }
}
