# Spec: Epic-04 — LP Content System (Learn / Practice)

Source: `Task/Epics/Epic-04_LPPP_Content_System_SPEC.md` (read-only client material)

> Citation note: this epic file's own section numbers skip one — §13
> "Acceptance Criteria (Epic-Level)" (line 927) is followed directly by §15
> "Mockups / Design References" (line 984); there is no §14 anywhere in the
> file (see Open questions). Every back-reference below therefore cites
> heading **text** plus a line number, never a bare section number.

## Problem

Epic-04 turns the platform from a pure scheduling tool into a
training-content ecosystem. Trainers cannot otherwise scale their expertise
beyond in-person sessions, players have no structured resources to use
between events, and trainers keep reinventing drills that other trainers
have already built ("Business Value — Problem Statement", line 44). Without
this epic the platform stays a commoditized scheduling tool with no
recurring engagement between events, no network effects from shared
content, and no way for trainers to scale training beyond in-person events
("Description — Business Value", lines 30-35).

The epic ships two MVP pillars — LEARN (tutorial videos with step-by-step
instruction) and PRACTICE (drills and workouts) — stated to be functionally
identical playlists of videos/drills plus text content, differing only in
their label, intended as a single implementation with a type/category field
("Description — Implementation Approach", lines 15-21). A third pillar,
PERFECT (feedback), and a fourth, PLAY (competition), are explicitly out of
MVP and deferred to Phase 2 ("Description", line 13; "Out of Scope
(Post-MVP) — Phase 2 Deferrals", line 103).

This epic depends on Epic-01 (players and trainers must already exist) and
Epic-02 (so content can be tied to events), per its own dependencies section
("Dependencies — Required Before This Epic", line 135); see Open questions
for a conflicting dependency claim involving Epic-05.

Four roles interact with the system: the Trainer/Business Owner creates
content and playlists and assigns them to players, and can discover and
reuse other trainers' public content; the Coach/Contractor can view a
trainer's content and their assigned players' progress but cannot create
content; the Player/Parent views and purchases only content from their own
connected trainer(s) and tracks their own progress; Super Admin can view
content and analytics across all trainers but cannot edit trainer content
("User Roles Involved", lines 149-156 — the table's own wording for two of
these roles still references the removed feedback pillar; see Open
questions, analyst-raised). Player access to content is gated by a paywall:
players can see the content library but must purchase access before they
can use it, under decision D-SCOPE-011 ("Business Rules & Logic — Content
Monetization Rules (Paywall Model - D-SCOPE-011)", line 700).

## User scenarios

1. **Trainer** creates a Learn playlist of tutorial videos so that players
   can learn fundamentals at their own pace.
   Path: Trainer opens "LPPP" → "Learn", clicks "+ Create Learn Playlist",
   enters a title, optional description, optional skill/position/age
   filters, and a Private/Public visibility setting; adds one or more
   YouTube videos each with a title, optional text instructions, tags, and
   duration; reorders them by drag-and-drop; and saves.
   Source: US-04.01 "Trainer Creates Learn Playlist" (line 162)

2. **Trainer** creates a drill with metadata in the Practice pillar so that
   they can organize drills and share them publicly.
   Path: Trainer opens "LPPP" → "Practice" → "Drill Database", clicks
   "+ Create New Drill", enters a name, optional text instructions, a
   required YouTube demonstration link, difficulty, equipment, space
   requirement, player count, duration range, one or more categories, and a
   visibility setting, then saves.
   Source: US-04.02 "Trainer Creates Drill in Practice Pillar" (line 209)

3. **Trainer** searches and discovers public drills from other trainers so
   that they can use proven drills without reinventing the wheel.
   Path: Trainer opens the Drill Database, toggles to "Public Drills,"
   searches and filters by category/difficulty/equipment/duration, previews
   a result, and clicks "Add to Playlist" to add it (by reference) to an
   existing or new Practice playlist, with the original creator recorded.
   Source: US-04.03 "Trainer Discovers and Uses Public Drill" (line 252)

4. **Trainer** creates a Practice playlist of multiple drills so that
   players can follow a structured workout sequence.
   Path: Trainer opens "LPPP" → "Practice", clicks "+ Create Practice
   Playlist", enters title/description/filters/visibility, adds drills
   either from the Drill Database (own or public) or by creating a new
   drill inline, optionally adds trainer notes/time per drill, reorders
   them, and saves.
   Source: US-04.04 "Trainer Creates Practice Playlist (Workout)" (line 299)

5. **Trainer** assigns a playlist to specific players or groups so that they
   receive structured training content.
   Path: From a playlist, trainer clicks "Assign to Players," selects
   players individually, by label, or by a filter (e.g. skill level),
   optionally sets a due date and note, and confirms; players are notified
   by email and in-app, and the assignment is tracked.
   Source: US-04.05 "Trainer Assigns Playlist to Players" (line 338)

6. **Trainer** marks a playlist or drill as Public so that other trainers
   can discover and use it.
   Path: Trainer toggles a content item's visibility from Private to
   Public, confirms, and the content appears in public discovery for all
   trainers with a "Public" badge, while the trainer retains ownership and
   can revert it later.
   Source: US-04.06 "Trainer Marks Content as Public" (line 371)

7. **Player or Parent** views and accesses training content from their
   trainer so that they can learn and practice between events.
   Path: Player opens the LPPP portal for their currently selected trainer,
   browses the Learn tab's content library (purchased and unpurchased, with
   locked indicators and suggested badges) and the Practice tab's assigned
   playlists, opens a video/drill player page showing the embedded video
   plus text instructions, and clicking play auto-marks the content
   complete.
   Source: US-04.07 "Player Views Assigned Content (LPPP Portal)" (line 400)

8. **Player or Parent** sees their training progress so that they stay
   motivated and see what they've completed.
   Path: Player opens "LPPP" → "Progress," sees overall stats (playlists
   assigned, items completed, watch time, completion rate), recent
   activity, incomplete assignments sorted by due date, and a per-playlist
   progress bar; trainers can see the same progress for a player from the
   CRM.
   Source: US-04.08 "Player Tracks Progress" (line 459)

9. **Trainer** edits an existing playlist so that they can update content
   and keep it current.
   Path: Trainer opens a playlist, clicks "Edit," changes title/
   description/filters/visibility/video-drill order, adds or removes
   items, and saves; already-assigned players see the update immediately,
   prior progress is unaffected, and new items show as not started.
   Source: US-04.09 "Trainer Edits Playlist" (line 496)

10. **Trainer** deletes a playlist or drill so that they can clean up
    unused content.
    Path: Trainer clicks "Delete" on a playlist (warned it will unassign it
    from all players) or a drill (warned how many playlists use it); on
    confirmation the item is removed from players' portals and, if Public,
    from public discovery, while player progress history and other
    trainers' broken references are handled as specified.
    Source: US-04.10 "Trainer Deletes Playlist or Drill" (line 525)

11. **Super Admin** views system-wide LPPP usage analytics so that they can
    understand platform engagement.
    Path: Super Admin opens "Super Admin" → "LPPP Analytics" and sees
    content stats, engagement stats, growth trends, and can drill down into
    a specific trainer's stats or export data to CSV.
    Source: US-04.11 "Super Admin Views LPPP Analytics" (line 556)

## Acceptance criteria

Every checkbox below is renumbered sequentially as AC-04-1 .. AC-04-43 from
the epic's per-story "Acceptance Criteria" lists (including their named
sub-blocks such as "Validation," "Attribution," and "Edge Cases"), one
criterion derived from an "In Scope (MVP)" scope bullet with no dedicated
story (Coach-Only Content), and the epic-level "Acceptance Criteria
(Epic-Level)" section. Where an epic-level criterion restates a story
criterion, the story version is kept and both origins are cited. Because
Learn and Practice are stated to be functionally identical (line 16),
criteria are written once where the epic itself states they apply to both
pillars, rather than duplicated. Checklist items that only restate the
removed PERFECT/feedback pillar are **not** carried forward as acceptance
criteria — see Open questions (analyst-raised).

**US-04.01 — Trainer Creates Learn Playlist** (line 162)
- [ ] **AC-04-1** Trainer can create a Learn playlist via "LPPP" → "Learn" →
  "+ Create Learn Playlist," entering a required title (max 100 characters),
  optional description, optional customization filters (skill level,
  position, age level — all multi-select), and a Private (default) or
  Public visibility toggle — `Epic-04_LPPP_Content_System_SPEC.md` §
  "US-04.01: Trainer Creates Learn Playlist" (lines 169-179; also epic-level
  AC, lines 932, 934, 935)
- [ ] **AC-04-2** For each video added to the playlist, the trainer supplies
  a YouTube URL, a required title, optional rich-text instructions (setup
  instructions, key points, additional explanations, post-video
  exercises/reflection questions), optional comma-separated tags, and a
  duration (auto-detected from YouTube if available, or entered manually);
  videos can be reordered by drag-and-drop, and saving creates the
  playlist, which appears in the Learn tab list with a confirmation message
  — § "US-04.01..." (lines 180-194; also epic-level AC, line 933)
- [ ] **AC-04-3** Learn playlist creation requires a non-empty title and at
  least one video, validates YouTube URL format, and allows the same video
  to appear more than once in a playlist (at different positions) — §
  "US-04.01..." § "Validation" (lines 197-200)

**US-04.02 — Trainer Creates Drill in Practice Pillar** (line 209)
- [ ] **AC-04-4** Trainer can create a drill (Practice pillar) with a
  required name (max 100 characters), optional rich-text instructions up to
  1000 characters (setup, execution steps, coaching points, common
  mistakes, variations), a required YouTube URL demonstrating the drill,
  difficulty level, equipment tags, space requirement, player count, a
  duration range (min/max minutes), one or more categories, and a Private
  (default) or Public visibility toggle — § "US-04.02: Trainer Creates
  Drill in Practice Pillar" (lines 219-234; also epic-level AC, lines 942,
  943)
- [ ] **AC-04-5** Saving a drill adds it to the trainer's "My Drills"
  section of the Drill Database, and if marked Public it also appears in
  public drill discovery for all trainers, with a confirmation message — §
  "US-04.02..." (lines 235-238; also epic-level AC, line 944)
- [ ] **AC-04-6** Drill creation requires a name and a valid-format YouTube
  URL, and requires at least one category — § "US-04.02..." § "Validation"
  (lines 241-243)

**US-04.03 — Trainer Discovers and Uses Public Drill** (line 252)
- [ ] **AC-04-7** Trainer can toggle from "My Drills" to "Public Drills" in
  the Drill Database and search/filter public drills by category,
  difficulty, equipment needed, and duration range; each result card shows
  the drill name, creator ("By [Trainer Name]"), video thumbnail,
  difficulty/duration/equipment badges, and a truncated description, with
  "Preview" and "Add to Playlist" actions — § "US-04.03: Trainer Discovers
  and Uses Public Drill" (lines 259-275; also epic-level AC, line 945)
- [ ] **AC-04-8** "Preview" opens the full drill details with the embedded
  video, equipment, duration, and categories, and an "Add to Playlist"
  action; "Add to Playlist" lets the trainer pick an existing Practice
  playlist or create a new one, adds the drill, stores a reference to the
  original creator, and confirms "Drill added to [Playlist Name]" — §
  "US-04.03..." (lines 276-285; also epic-level AC, lines 946, 955)
- [ ] **AC-04-9** Search matches drill name, description, and tags; filters
  combine with AND logic; an empty result set shows "No drills match your
  filters. Try adjusting criteria." — § "US-04.03..." § "Search/Filter
  Logic" (lines 288-290)
- [ ] **AC-04-10** The original creator of a drill is tracked in the
  database and shown to trainers in the drill list (admin view);
  player-facing attribution is deferred to Phase 2 — § "US-04.03..." §
  "Attribution" (lines 293-295)

**US-04.04 — Trainer Creates Practice Playlist (Workout)** (line 299)
- [ ] **AC-04-11** Trainer can create a Practice playlist with a required
  title, optional description, optional customization filters
  (skill/position/age), and a Private/Public visibility toggle, then add
  drills either by searching/selecting from the Drill Database (own and
  public drills) or by creating a new drill inline — § "US-04.04: Trainer
  Creates Practice Playlist (Workout)" (lines 309-319; also epic-level AC,
  line 947)
- [ ] **AC-04-12** Drills within a Practice playlist can be reordered by
  drag-and-drop and removed; for each drill the trainer can optionally add
  trainer notes and an estimated time overriding the drill's default;
  saving creates the playlist, which appears in the Practice tab list — §
  "US-04.04..." (lines 320-327)

**US-04.05 — Trainer Assigns Playlist to Players** (line 338)
- [ ] **AC-04-13** Trainer can assign a playlist to players by individual
  multi-select, by label group, or by filtering players (e.g. by skill
  level/age), with the selected count shown; the trainer may optionally set
  a due date and an assignment note visible to players — § "US-04.05:
  Trainer Assigns Playlist to Players" (lines 345-353; also epic-level AC,
  line 936)
- [ ] **AC-04-14** Assigning a playlist notifies the players by email and
  in-app ("New content assigned: [Playlist Name]" with a link to the LPPP
  portal), the playlist appears in the players' LPPP portal, and the
  assignment records who assigned it, when, and to whom — § "US-04.05..."
  (lines 354-359; also epic-level AC, line 969)
- [ ] **AC-04-15** Trainer can view assignment status for a playlist as
  counts (e.g. "Assigned to 15 players, 8 completed, 7 in progress") and
  click through to see individual player progress — § "US-04.05..." §
  "Assignment Status" (lines 362-363)
- [ ] **AC-04-16** The same playlist can be assigned to different players
  or groups at any time, with no limit on the number of assignments — §
  "US-04.05..." § "Reassignment" (lines 366-367)

**US-04.06 — Trainer Marks Content as Public** (line 371)
- [ ] **AC-04-17** Trainer can toggle any of their own content (playlist or
  drill) between Private and Public from the playlist list or Drill
  Database; switching to Public shows a confirmation ("Make [Content]
  public? Other trainers will be able to discover and use it.") while the
  trainer retains ownership and can revert to Private at any time — §
  "US-04.06: Trainer Marks Content as Public" (lines 378-382; also
  epic-level AC, line 952)
- [ ] **AC-04-18** Content marked Public appears in public discovery for
  all trainers with a green "Public" badge — § "US-04.06..." (lines
  383-384; also epic-level AC, line 953)
- [ ] **AC-04-19** The original creator of content is stored and shown to
  other trainers as "By [Trainer Name]" in admin views; if the creator
  later switches the content back to Private, it is removed from public
  discovery but existing playlist references from other trainers keep
  working — § "US-04.06..." § "Content Attribution (per Q-P1-016)" (lines
  387-391 — the "Q-P1-016" citation does not resolve within this file; see
  Open questions, analyst-raised — also epic-level AC, lines 954, 956, 968)

**US-04.07 — Player Views Assigned Content (LPPP Portal)** (line 400)
- [ ] **AC-04-20** Navigating to "LPPP" shows content for the player's
  currently selected trainer context only, and the LPPP portal opens with
  tabs — the epic-level acceptance criteria list these as Learn, Practice,
  and Progress (line 959), while the story itself lists Learn, Practice,
  Perfect, and Progress (line 409); the "Perfect" tab is not carried
  forward here since the PERFECT pillar is out of MVP scope — see Open
  questions (analyst-raised) — § "US-04.07: Player Views Assigned Content
  (LPPP Portal)" (lines 407-409)
- [ ] **AC-04-21** The Learn tab's Content Library shows all available
  Learn playlists, both purchased and unpurchased, with a locked indicator
  for unpaid content and a "Suggested" badge for trainer-recommended
  content; each playlist shows its title, description, trainer name,
  price/purchase status ("Free," a price, or "Subscribed"), locked/unlocked
  status, and progress if purchased (e.g. "3 of 5 videos completed") — §
  "US-04.07..." § "Learn Tab" (lines 412-421; also "In Scope (MVP) — Player
  Experience," lines 91-92; also epic-level AC, line 960)
- [ ] **AC-04-22** Clicking a locked playlist opens the purchase/payment
  flow (Epic-05); clicking an unlocked or purchased playlist opens its
  detail view listing videos in order, each showing title, duration, and a
  completion checkmark, with each video opening the video player page — §
  "US-04.07..." § "Learn Tab" (lines 422-426; also "In Scope (MVP) — Player
  Experience," line 94)
- [ ] **AC-04-23** The Practice tab lists assigned Practice playlists
  (workouts), each showing title, description, who assigned it, due date,
  and progress (e.g. "2 of 4 drills completed"); opening a playlist shows
  its drills in order, each with title, duration, equipment, a completion
  checkmark, and any trainer notes, with each drill opening the drill
  player page — § "US-04.07..." § "Practice Tab" (lines 429-438; also
  epic-level AC, line 948)
- [ ] **AC-04-24** The video/drill player page shows the embedded YouTube
  player, the content title, standard playback controls (play, pause, seek,
  volume), and a prominently displayed Text Instructions section (setup
  instructions, key points, coaching notes) formatted with bullet/numbered
  lists and expandable/collapsible if long — § "US-04.07..." §
  "Video/Drill Player Page" (lines 441-447; also epic-level AC, line 961)
- [ ] **AC-04-25** Content is automatically marked complete when the player
  clicks play, with a completion checkmark shown immediately and a "Next"
  control to advance to the next video/drill in the playlist — §
  "US-04.07..." § "Video/Drill Player Page — Auto-completion" (lines
  448-450; also epic-level AC, lines 938, 949) — this specs the story's
  auto-complete mechanism; see Open questions (analyst-raised) for its
  conflict with the "Mark content as complete" scope bullet.
- [ ] **AC-04-26** A parent can switch between children via a dropdown to
  view each child's assigned content separately; auto-completion on play
  works the same way for a child's content — § "US-04.07..." § "Parent
  Behavior (Child Account)" (lines 453-455)

**US-04.08 — Player Tracks Progress** (line 459)
- [ ] **AC-04-27** The Progress tab dashboard shows overall stats: total
  playlists assigned, total videos/drills completed, total watch time, and
  a completion rate (completed/total assigned) — § "US-04.08: Player Tracks
  Progress" § "Overall Stats" (lines 470-473; also epic-level AC, lines 939,
  962, 963)
- [ ] **AC-04-28** The dashboard lists recent activity (recently completed
  content with title, pillar, and completion date) and incomplete
  assignments (assigned playlists not yet completed, sorted by nearest due
  date first, clickable to open the playlist) — § "US-04.08..." § "Recent
  Activity" and "Incomplete Assignments" (lines 476-483)
- [ ] **AC-04-29** For each assigned playlist, the dashboard shows a
  progress bar and a count (e.g. "3 of 5 items completed") — §
  "US-04.08..." § "Progress per Playlist" (lines 486-488; also epic-level
  AC, line 963)
- [ ] **AC-04-30** A trainer can view a player's progress from the CRM
  player detail (Epic-03), showing assigned playlists, completion status,
  and last activity — § "US-04.08..." § "Trainer View of Player Progress"
  (lines 491-492)

**US-04.09 — Trainer Edits Playlist** (line 496)
- [ ] **AC-04-31** Trainer can edit an existing playlist's title,
  description, customization filters, visibility, and video/drill order
  (drag-and-drop), and can add or remove videos/drills; saving shows a
  confirmation "Playlist updated!" — § "US-04.09: Trainer Edits Playlist"
  (lines 503-512, 517)
- [ ] **AC-04-32** If the edited playlist is already assigned to players,
  they see the updated content immediately; existing completion progress is
  not affected by the edit, and any newly added items appear as "not
  started" — § "US-04.09..." (lines 513-516)
- [ ] **AC-04-33** If a video/drill is removed from a playlist, a player's
  prior progress on that item remains for history; if items are reordered,
  player progress is unaffected — § "US-04.09..." § "Edge Cases" (lines
  520-521; also epic-level AC, line 967)

**US-04.10 — Trainer Deletes Playlist or Drill** (line 525)
- [ ] **AC-04-34** Trainer can delete a playlist via a trash-icon action; a
  confirmation warns "Delete [Playlist Name]? This will unassign it from
  all players"; on confirmation the playlist is removed from assigned
  players' LPPP portal and (if Public) from public discovery, while player
  progress history on it is preserved for trainer analytics; if other
  trainers were using it, their references break and show "Content
  unavailable" — § "US-04.10: Trainer Deletes Playlist or Drill" §
  "Acceptance Criteria - Delete Playlist" (lines 532-538)
- [ ] **AC-04-35** Trainer can delete a drill via a trash-icon action; if
  the drill is used in playlists, a warning states how many playlists use
  it ("This drill is used in [N] playlists. Delete anyway?") and confirms
  it will be removed from those playlists; on confirmation the drill is
  deleted and (if Public) removed from public discovery, and other
  trainers' playlist references to it show "Drill unavailable" — §
  "US-04.10..." § "Acceptance Criteria - Delete Drill" (lines 541-547)
- [ ] **AC-04-36** (Recommendation, not a strict criterion) The epic
  recommends that playlist and drill deletion be implemented as a soft
  delete — marked deleted and hidden from the UI rather than removed from
  the database — to preserve data for analytics and history — §
  "US-04.10..." § "Soft Delete Recommendation (Technical)" (lines 550-552)

**US-04.11 — Super Admin Views LPPP Analytics** (line 556)
- [ ] **AC-04-37** Super Admin's LPPP Analytics dashboard shows content
  stats: total playlists created (all trainers), total drills created, the
  public-vs-private ratio, the top 10 most-used public drills, and the top
  content creators (trainers with the most public content) — § "US-04.11:
  Super Admin Views LPPP Analytics" § "Content Stats" (lines 567-571)
- [ ] **AC-04-38** The dashboard shows engagement stats: total player video
  views (all-time and this week), average watch time per player, and
  completion rate — § "US-04.11..." § "Engagement Stats" (lines 574-576).
  The source also lists two feedback-request metrics here (lines 577-578),
  not carried forward since they reflect the removed PERFECT pillar — see
  Open questions (analyst-raised).
- [ ] **AC-04-39** The dashboard shows growth trends: content-creation
  trend (playlists/drills per week), player-engagement trend (views per
  week), and public-content growth (new public items per week) — §
  "US-04.11..." § "Growth Trends" (lines 581-583)
- [ ] **AC-04-40** Super Admin can drill down into a specific trainer's
  LPPP stats and export the data to CSV — § "US-04.11..." § "Drill-Down"
  (lines 586-587)

**Scope note (not a dedicated user story)**
- [ ] **AC-04-41** Trainers can create playlists visible only to coaches,
  not players, intended for coach certifications, training methodology, or
  internal videos, via a visibility toggle in playlist settings —
  `Epic-04_LPPP_Content_System_SPEC.md` § "In Scope (MVP) — Content
  Management (Cross-Pillar) — Coach-Only Content" (lines 81-84). The epic
  does not detail this in a dedicated user story, business rule, or Data
  Requirements field — the Playlist visibility field is modeled as binary
  Public/Private only (line 601); see Open questions (analyst-raised).

**Epic-level (from "Acceptance Criteria (Epic-Level)," line 927) — not
already covered above**
- [ ] **AC-04-42** Performance targets: the LP portal loads in under 2
  seconds and drill-database search returns results in under 1 second;
  YouTube videos are expected to embed and play smoothly — § "Acceptance
  Criteria (Epic-Level) — Performance" (lines 972-974). The narrative
  "Performance & Scale Targets" section states two further targets
  (playlist detail load, video player load) not repeated in this checklist
  — see Open questions (analyst-raised).
- [ ] **AC-04-43** (Process gate, not product behavior) Epic-04 is
  considered complete only once the demo is approved, all P0 open questions
  are resolved, public content network effects are validated, and the
  YouTube embedding strategy is confirmed working — § "Acceptance Criteria
  (Epic-Level) — Approval" (lines 977-980)

**Final count: 43 acceptance criteria (AC-04-1 .. AC-04-43)**, covering all
11 user stories (US-04.01–US-04.11), one scope note with no dedicated story
(Coach-Only Content), and the epic-level completion checklist. 29 of the
epic-level section's 37 checkable items are folded into their matching
story criterion above rather than duplicated; the Performance (3 items) and
Approval (4 items) subsections became two new dedicated criteria (AC-04-42,
AC-04-43) since they have no per-story equivalent; one remaining item
("Playlists and content stored correctly," line 966) is a generic
restatement with no independently testable content and is not separately
cited.

## Business rules

Restated from "Business Rules & Logic" (line 660), organized by its named
rule groups:

**YouTube Video Strategy** (line 662)
- [ ] **BR-04-1** All videos are hosted externally on YouTube; the platform
  stores only YouTube URLs and does not store video files. Videos are
  uploaded to YouTube by Dale and by trainers, separately from the
  platform, and are set to Unlisted (accessible only via link, not publicly
  searchable) — `Epic-04_LPPP_Content_System_SPEC.md` § "Business Rules &
  Logic — YouTube Video Strategy — Video Hosting" (lines 665-668)
- [ ] **BR-04-2** Videos are displayed in an embedded YouTube player
  supporting play, pause, seek, volume, and fullscreen; YouTube itself
  handles streaming, transcoding, CDN delivery, and adaptive quality — §
  "...YouTube Video Strategy — Embedding" (lines 671-673)

**Content Visibility Rules** (line 681)
- [ ] **BR-04-3** Private content (the default for new content) is visible
  only to its creating trainer, who can assign it to their own players; it
  is not discoverable by other trainers — § "...Content Visibility Rules —
  Private Content" (lines 684-687)
- [ ] **BR-04-4** Public content is visible to all trainers in public
  discovery (search/filter) and any trainer can add it to their own
  playlists; the original creator retains ownership and can revert it to
  Private at any time — § "...Content Visibility Rules — Public Content"
  (lines 690-693)
- [ ] **BR-04-5** Switching public content back to Private removes it from
  public discovery, but other trainers' existing playlist references to it
  keep working (don't break), preserving content continuity for players who
  were already using it — § "...Content Visibility Rules — Public to
  Private Transition" (lines 696-698)

**Content Monetization Rules (Paywall Model - D-SCOPE-011)** (line 700)
- [ ] **BR-04-6** Under the paywall model (decision D-SCOPE-011): players
  can see the content library (playlists, videos) but cannot access content
  without paying; a trainer may suggest specific content to players;
  players must purchase access to view content; payment is processed
  through Epic-05 as trainer-specific tokens, a subscription, or a
  one-time payment — § "...Content Monetization Rules (Paywall Model -
  D-SCOPE-011) — Paywall Model" (lines 703-706)
- [ ] **BR-04-7** Unpaid content is shown with a locked indicator; clicking
  locked content opens the payment/purchase flow; after purchase the
  player has full access to the content, and access persists under either a
  one-time-purchase model or a subscription-based model — § "...Paywall
  Model - D-SCOPE-011 — Access Control" (lines 709-712)
- [ ] **BR-04-8** The pricing structure is explicitly "to be finalized with
  client" and names four candidate options, none yet selected: per-playlist
  one-time pricing, an all-content monthly subscription, bundle pricing
  across multiple playlists at a discount, or inclusion with an event
  subscription — § "...Paywall Model - D-SCOPE-011 — Pricing Structure (To
  be finalized with client)" (lines 714-718) — see Open questions
  (analyst-raised).
- [ ] **BR-04-9** A trainer can suggest specific playlists to specific
  players; the suggestion appears as a notification and is highlighted in
  the player's LPPP portal; the player must still purchase access if
  paywall is enabled for that content, but free content can be suggested
  without a purchase requirement — § "...Paywall Model - D-SCOPE-011 —
  Trainer Content Suggestions" (lines 721-724)

**Multi-Trainer Content Visibility** (line 726)
- [ ] **BR-04-10** Players see the content library for their currently
  selected trainer context only; switching trainer context shows a
  different content library; each trainer's content is completely
  isolated; a player can purchase content separately from each trainer they
  are connected to, consistent with the platform's separated-views
  architecture for events, tokens, and content — § "...Multi-Trainer
  Content Visibility — Separated Views for Players with Multiple Trainers"
  (lines 729-733)

**Content Attribution Rules** (line 735)
- [ ] **BR-04-11** Every content item stores its original creator; the
  creator's name is shown to other trainers in admin views (e.g. "By Coach
  Sarah"); player-facing attribution is deferred to Phase 2 — §
  "...Content Attribution Rules — Original Creator Tracking" (lines
  738-740)
- [ ] **BR-04-12** When a trainer adds another trainer's public content —
  including drills — to their own playlist, a reference is stored rather
  than a copy; if the original creator later deletes that content, the
  reusing trainer's reference breaks and shows "Unavailable" — §
  "...Content Attribution Rules — Content Reuse" (lines 743-745; restated
  specifically for drills at "...Drill Database Rules — Drill Reuse," lines
  798-800)

**Playlist Assignment Rules** (line 752)
- [ ] **BR-04-13** A trainer can assign a playlist to individual players,
  to an entire label group (e.g. "Beginners"), or to players filtered by an
  attribute such as skill level — § "...Playlist Assignment Rules —
  Assignment Targets" (lines 755-757)
- [ ] **BR-04-14** The same playlist can be assigned to different players
  or groups multiple times, with each assignment tracked separately — §
  "...Playlist Assignment Rules — Multiple Assignments" (lines 760-761)
- [ ] **BR-04-15** A playlist can be reassigned to a player who already has
  it, refreshing the assignment while keeping that player's existing
  progress (not reset) — § "...Playlist Assignment Rules — Reassignment"
  (lines 764-765)

**Progress Tracking Rules** (line 767)
- [ ] **BR-04-16** Content is automatically marked complete when a player
  starts watching or viewing it, triggered by clicking play or opening the
  content (engagement started) — not by a manual "Mark as Complete" action,
  which the epic states is not needed; a timestamp is recorded and the
  progress bar updates accordingly — § "...Progress Tracking Rules —
  Video/Drill Completion" and "— Completion Criteria" (lines 770-773,
  776-778) — contradicted by an in-scope MVP bullet; see Open questions
  (analyst-raised).
- [ ] **BR-04-17** Progress is preserved if content is removed from a
  playlist (kept for history) or if a playlist is deleted (the trainer can
  still view it in analytics); if a playlist is reassigned to the same
  player, their existing progress is kept, not reset — § "...Progress
  Tracking Rules — Progress Persistence" (lines 781-783)

**Drill Database Rules** (line 785)
- [ ] **BR-04-18** Trainers create drills with metadata, stored in their
  personal drill database with a default visibility of Private — §
  "...Drill Database Rules — Drill Creation" (lines 788-790)
- [ ] **BR-04-19** Trainers can search and filter public drills by
  category, difficulty, equipment, and duration, and can preview a drill
  before adding it to a playlist — § "...Drill Database Rules — Public
  Drill Discovery" (lines 793-795)

## Data requirements

Restated from "Data Requirements — What Information Needs to Be Stored"
(line 593) — entities, the fields the epic names, and the relationships it
names. No schema, keys, or types are proposed here.

- **Playlist**: unique identifier, pillar (Learn or Practice), trainer who
  created it, title, description, customization filters (skill, position,
  age levels), visibility (Public or Private), created/updated timestamps,
  and — if public — a public date. Relationship: created by a Trainer;
  contains Content Items via the Playlist-Content Association. (lines
  595-603)
- **Content Item** (Video or Drill, common fields): unique identifier,
  trainer who created it (original creator), pillar, type, title (short,
  max 100 characters), text instructions/content (rich text up to 1000
  characters, supporting bullet points and numbered lists — setup
  instructions, key points, coaching notes), YouTube URL (unlisted link),
  duration (seconds), visibility (Public or Private), tags (array of
  strings), created/updated timestamps. The epic's own field list for
  pillar and type still includes the removed "Perfect" pillar and a
  "feedback" type (lines 608-609) — not carried into this spec as real MVP
  values; see Open questions (analyst-raised). Relationship: created by a
  Trainer; associated with Playlists via the Playlist-Content Association.
  (lines 605-618)
- **Drill** (Practice-pillar-specific fields, in addition to Content Item's
  common fields): difficulty level (Beginner, Intermediate, Advanced,
  Elite), equipment needed (array, e.g. Ball, Cones, Ladder), space
  requirement (Small/Medium/Large), player count (text, e.g. "1-2," "2-4"),
  duration range (min/max minutes), categories (array, e.g. Dribbling,
  Shooting, Passing). Relationship: extends a Content Item. (lines 620-627)
- **Playlist-Content Association** (many-to-many): which playlist, which
  content item, sequence order (position in playlist), optional "is
  required" flag, trainer notes specific to that playlist context.
  Relationship: join entity between Playlist and Content Item. (lines
  629-634)
- **Content Assignment**: which playlist, which player(s), who assigned it
  (trainer or coach), assigned timestamp, optional due date, optional
  assignment note (visible to the player), status (not started, in
  progress, completed). Relationship: references a Playlist and one or more
  Players; records who assigned it. (lines 636-643)
- **Content Progress**: which content item, which player, status (not
  started, in progress, completed), progress percentage (0-100, for video
  watch progress), completed timestamp, watch time (seconds spent on the
  content). Relationship: references a Content Item and a Player. (lines
  645-651)
- **Content Attribution** (public content): original creator (trainer
  reference), optionally an array of trainers using the content (for
  analytics), usage count (how many trainers added it to a playlist).
  Relationship: references the creating Trainer and, optionally, reusing
  Trainers. (lines 653-656)

## Edge cases

| Case | Expected |
|---|---|
| Trainer switches content from Public back to Private | Removed from public discovery; other trainers' existing playlist references to it keep working (don't break) — Business Rules "Content Visibility Rules — Public to Private Transition" (lines 696-698); also US-04.06 (line 391) |
| Original creator deletes content (drill or other) that other trainers have added to their own playlists | The reusing trainers' references break, shown as "Unavailable" / "Drill unavailable" — Business Rules "Content Attribution Rules — Content Reuse" (lines 743-745) and "Drill Database Rules — Drill Reuse" (line 800); US-04.10 (line 547) |
| Trainer deletes a playlist that other trainers are also using | Their references break and show "Content unavailable"; the deleting trainer's own assigned players lose the playlist from their portal, but their progress history on it is preserved for analytics — US-04.10 "Acceptance Criteria - Delete Playlist" (lines 535-538) |
| Trainer deletes a drill that is used in one or more playlists | A warning states how many playlists use it ("This drill is used in [N] playlists. Delete anyway?"); confirming removes the drill from those playlists — US-04.10 "Acceptance Criteria - Delete Drill" (lines 542-544) |
| A playlist is reassigned to a player who already has progress on it | The assignment refreshes but existing progress is kept, not reset — Business Rules "Playlist Assignment Rules — Reassignment" (lines 764-765) and "Progress Tracking Rules — Progress Persistence" (line 783); Testing Considerations (line 1023) |
| A content item is removed from a playlist that players already have progress on | The player's progress on that item is preserved for history — US-04.09 "Edge Cases" (line 520) |
| A playlist's items are reordered | Player progress is unaffected by the reorder — US-04.09 "Edge Cases" (line 521) |
| Player clicks play on the same video/drill more than once | Completion stays idempotent — Testing Considerations "Edge Cases" (line 1022) |
| Player completes a playlist after its due date has passed | The completion is still recorded — Testing Considerations "Edge Cases" (line 1025) |
| Public drill search/filter matches nothing | "No drills match your filters. Try adjusting criteria." is shown — US-04.03 "Search/Filter Logic" (line 290); also a general empty-state case in Testing Considerations (line 1026) |
| Trainer enters an invalid-format YouTube URL | Rejected by format validation — US-04.01 "Validation" (line 199); US-04.02 "Validation" (line 242); Testing Considerations (line 1020) |
| Trainer adds the same video to a Learn playlist more than once | Allowed; the same video may appear at more than one position in the playlist — US-04.01 "Validation" (line 200) |

Boundary/failure behaviors the epic raises but does **not** state an
expected outcome for (the YouTube video/player failing to load, e.g. due to
YouTube API issues; what happens if YouTube-based duration auto-detection
is unavailable at video-add time) are listed in Open questions instead of
guessed here — Testing Considerations "Edge Cases" (line 1021); US-04.01
(line 190).

## Out of MVP scope

Verbatim from "Out of Scope (Post-MVP)" (line 100):

**Phase 2 Deferrals** (line 102):
- PERFECT pillar (Feedback Layer) - Player submits video link, trainer
  provides feedback - removed from MVP
- PLAY pillar (competition layer)
- Direct video uploads (platform uses external video hosting links only)
- Video feedback (trainer response videos)
- AI-powered feedback analysis
- Video annotations and drawing tools
- Player-facing content attribution ("Drill by Coach Sarah")
- Usage analytics for public content creators
- Content licensing or revenue sharing
- Workout templates (pre-built sequences)
- Scheduled content release (content available immediately)
- Content recommendations (AI-suggested playlists)
- Player video comparison (side-by-side before/after)
- Leaderboards (player rankings by content completion)
- Badges and achievements
- Social features (player comments, likes, shares)
- Content versioning (edit history, rollback)
- Content export (download playlists)
- Multi-language content
- Content moderation (flagging inappropriate content)

**Simplifications Applied** (line 124):
- Learn + Practice consolidated (same functionality, different labels only)
- Advanced content organization removed
- Player progress tracking simplified (auto-complete when player starts
  watching, no manual button)
- Public library curation removed (light-touch only)
- Content engagement analytics removed

## Open questions

### From the epic's "Questions / Open Issues" section (line 916, verbatim)

| ID | Question | Priority | Status | Owner |
|:---|:---|:---:|:---|:---|
| Q-04.03 | Content moderation: How to handle inappropriate public content? Flag system? | P2 | Open | Team |
| Q-04.06 | Playlist due dates: Reminder notifications 24 hours before? | P2 | Open | Client |
| Q-04.09 | Can trainers schedule playlist assignments (release on specific date)? | P2 | Open | Client |
| Q-04.10 | Content versioning: If trainer edits public content, notify trainers using it? | P2 | Open | Team |

### Analyst-raised

- **(analyst-raised)** Dependency cycle with Epic-05. Epic-04 §5
  "Dependencies — Required Before This Epic" (line 135) lists only Epic-01
  and Epic-02 as required before Epic-04. But Epic-05 §5 "Dependencies —
  Required Before This Epic" (`Epic-05_Payments_Tokens_SPEC.md`, line 148)
  lists Epic-04 as required before Epic-05 ("Content must exist to
  purchase"), and `Epic_Areas_Plan.md`'s own Epic-04 section lists
  "Epic-05: Payment Processing (content purchases)" under its
  "### Dependencies" heading (`Epic_Areas_Plan.md`, line 318) — implying
  Epic-04 needs Epic-05, not the reverse. Content purchase is explicitly in
  Epic-04's MVP scope ("In Scope (MVP) — Player Experience," line 94: "
  Purchase content access (integration with Epic-05)"). This spec does not
  resolve the cycle; that is an architect-stage decision.
- **(analyst-raised)** Progress tracking contradicts itself inside this
  epic. "In Scope (MVP) — Player Experience" (line 95) lists "Mark content
  as complete" as a player action — implying a manual action. "Out of
  Scope (Post-MVP) — Simplifications Applied" (line 127) says the opposite:
  progress is "auto-complete when player starts watching, no manual
  button." US-04.08 "Player Tracks Progress" (line 459) itself only
  specifies the progress *dashboard* (stats, recent activity, progress
  bars) and states no completion mechanism at all. The actual mechanism is
  spelled out in US-04.07's "Video/Drill Player Page — Auto-completion"
  (line 448: "When player clicks play → Content automatically marked
  complete") and in Business Rules "Progress Tracking Rules — Completion
  Criteria" (line 777: "No manual 'Mark as Complete' button needed"). This
  spec follows the more detailed, twice-restated auto-complete version
  (AC-04-25, BR-04-16) and flags the "Mark content as complete" scope
  bullet (line 95) as stale/contradicted rather than silently dropping it.
- **(analyst-raised)** Stale role table. "User Roles Involved" (line 149)
  still says the Trainer/Business Owner role "...provides feedback" (line
  153) and the Player/Parent role "...submits feedback requests" (line
  155) — that is the PERFECT pillar, explicitly removed from MVP
  ("Description," line 13; "Out of Scope (Post-MVP)," line 103). Feedback
  criteria from this table are not carried into this spec's Problem,
  Acceptance criteria, or Business rules.
- **(analyst-raised)** The PERFECT/feedback staleness above is not
  confined to the role table; it recurs in three further places, none of
  which are carried forward as real MVP behavior in this spec: (a)
  US-04.07's own acceptance criteria list "Perfect" as one of four LPPP
  portal tabs (line 409), directly conflicting with the epic-level
  acceptance criteria's three-tab list "Learn, Practice, Progress tabs"
  (line 959); (b) the Data Requirements "For Content Items" field list
  still offers "Perfect" as a Pillar value and "feedback" as a content Type
  value (lines 608-609); (c) Super Admin Analytics' "Engagement Stats"
  still count "Feedback requests submitted" and "Feedback requests
  completed (response rate)" (lines 577-578).
- **(analyst-raised)** The paywall pricing structure is explicitly
  unresolved in the source itself: "Business Rules & Logic — Content
  Monetization Rules (Paywall Model - D-SCOPE-011) — Pricing Structure"
  states "(To be finalized with client)" and lists four candidate options
  — per-playlist one-time purchase, an all-content subscription, bundle
  pricing, or inclusion with an event subscription — without selecting one
  (lines 714-718). This spec captures all four as BR-04-8 without resolving
  them.
- **(analyst-raised)** "Coach-Only Content" (playlists visible only to
  coaches, not players, for certifications/methodology/internal videos) is
  asserted once as an MVP in-scope bullet ("In Scope (MVP) — Content
  Management (Cross-Pillar)," lines 81-84) and once more in
  `Epic_Areas_Plan.md` (lines 277, 295), but has no dedicated user story,
  no business rule, and no Data Requirements field: the Playlist visibility
  field is modeled as binary Public/Private only (line 601), not a
  three-state Public/Private/Coach-only value. Spec'd here as a standalone
  criterion derived from the scope note (AC-04-41), but the underlying
  visibility model and the intended audience check (how the system knows a
  viewer is "a coach" versus "a player") are unresolved.
- **(analyst-raised)** Despite the epic's statement that Learn and Practice
  are functionally identical (line 16), two places only spell out one
  pillar's detail: (a) public content discovery/reuse is only detailed for
  drills (US-04.03: search, filter, preview, "Add to Playlist"); no story
  gives the equivalent flow for discovering and reusing a whole public
  Learn or Practice playlist, even though playlists carry their own
  Public/Private field (line 601) and the Content Visibility/Content Reuse
  business rules speak generically about "content." (b) The Learn Tab
  acceptance criteria spell out locked indicators, price/purchase status,
  and a click-to-purchase flow (lines 413-422); the Practice Tab acceptance
  criteria (lines 429-438) have no equivalent paywall fields, even though
  the Content Monetization Rules describe the paywall generically over the
  "content library (playlists, videos)" without excluding Practice.
  Unclear whether Practice content is meant to always be free or the story
  detail was simply omitted.
- **(analyst-raised)** Performance targets are stated in two places that
  don't match. The narrative "Performance & Scale Targets" section (lines
  906-910) includes "Playlist detail load: <1 second" and "Video player
  load: <2 seconds," neither of which appears in the epic-level
  "Acceptance Criteria (Epic-Level) — Performance" checklist (lines
  972-974); that checklist's "YouTube videos embed and play smoothly"
  (line 974) does not appear in the narrative section either. Worth
  reconciling into one list (mirrors a similar split found in Epic-01).
- **(analyst-raised)** An inline reference "(per Q-P1-016)" appears under
  US-04.06's "Content Attribution" criteria (line 386), naming a question
  ID that follows neither this epic's own Q-04.xx numbering nor appears in
  its "Questions / Open Issues" table (lines 916-923); it cannot be
  resolved from this file alone.
- **(analyst-raised)** The "Questions / Open Issues" table's ID sequence is
  sparse: only Q-04.03, Q-04.06, Q-04.09, and Q-04.10 are present (lines
  920-923); Q-04.01, Q-04.02, Q-04.04, Q-04.05, Q-04.07, and Q-04.08 do not
  appear anywhere in the file. Unclear whether these were answered and
  removed, reserved, or never filled in.
- **(analyst-raised, methodology note)** Section numbering skips: the
  document's headings go from "## 13. Acceptance Criteria (Epic-Level)"
  (line 927) directly to "## 15. Mockups / Design References" (line 984)
  — there is no "## 14." heading anywhere in the file. (Contrast with
  Epic-01, where sections were duplicated rather than skipped.) Worth a fix
  request to the client if this file is revised.
- **(analyst-raised)** No expected fallback behavior is stated for a
  YouTube video/player failing to load (e.g. due to YouTube API issues) —
  Testing Considerations lists it as a scenario to test (line 1021) but
  neither that section nor any user story says what the player should see
  or be able to do when it happens.
