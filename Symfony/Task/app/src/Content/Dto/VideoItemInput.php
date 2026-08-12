<?php

declare(strict_types=1);

namespace App\Content\Dto;

/**
 * One video row inside `LearnPlaylistType`'s embedded collection (AC-04-2).
 * Learn-playlist videos are always created inline/new — unlike drills, no
 * story in this epic describes reusing an existing video `ContentItem`
 * across playlists (see Open questions, analyst-raised, on this exact gap).
 *
 * @see specs/requirements-analyst-epic-04-lp-content-spec.md AC-04-2/3
 */
final readonly class VideoItemInput
{
    /**
     * @param list<string>|null $tags
     */
    public function __construct(
        public string $youtubeUrl,
        public string $title,
        public ?string $instructions,
        public ?array $tags,
        public ?int $durationSeconds,
    ) {
    }
}
