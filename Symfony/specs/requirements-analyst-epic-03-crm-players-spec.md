# Spec: Epic-03 — CRM & Player Management

Source: `Task/Epics/Epic-03_CRM_Player_Management_SPEC.md` (read-only client material)

> Citation note: this epic file's top-level section numbering skips §14 — it
> jumps from §13 "Acceptance Criteria (Epic-Level)" (line 926) straight to
> §15 "Mockups / Design References" (line 982); no §14 appears anywhere in
> the file (see Open questions). Every back-reference below therefore cites
> heading **text** plus a line number, never a bare section number.

## Problem

Epic-03 turns the platform from a pure event-scheduling tool into a full CRM
for trainers: labels, system-defined flags, notes, segmentation/filtering,
private-event targeting, and a Quick View dashboard sit on top of the player
and attendance data the platform already collects ("Description", lines
7-9). The epic states its own problem plainly: trainers today track player
information with spreadsheets, paper notes, and memory, which the epic says
leads to missed engagement opportunities, poor retention, and an inability
to spot trends or segment players for targeted communications ("Business
Value — Problem Statement", lines 24-25). It frames this as five concrete
losses if the epic doesn't ship: trainers cannot segment players for
targeted communications, cannot track engagement or identify at-risk
players, cannot identify top performers or scholarship candidates, have no
business intelligence for growth decisions, and have limited ability to
personalize training ("Description — Business Value", lines 12-16).

Four roles touch the CRM differently, and the epic is explicit that the
difference is by design: the Trainer/Business Owner gets full CRM access
scoped to their own organization only; the Coach/Contractor is limited to
players tied to the coach's own assigned events; the Player/Parent has no
CRM access at all (their own profile stays Epic-01's concern); and the
Super Admin gets full cross-trainer access, including applying flags across
trainers ("User Roles Involved", lines 118-123). The epic requires Epic-01
(so players, coaches, and trainers already exist) and Epic-02 (so
attendance data exists to build segmentation and the Top Players algorithm
on) before it can be built, and once built it unblocks Epic-04 (which can
target LPPP content at the player segments this epic creates) and Epic-06
(which consumes the ShareLink tracking this epic records)
("Dependencies — Required Before This Epic" and "— Blocks These Epics",
lines 102-108).

## User scenarios

1. **Trainer** views their full player list so that they can manage their
   player base.
   Path: Trainer opens "CRM Tools"/"Players" and sees every player
   associated with their organization — sourced from ShareLink invitations,
   event registrations, and coach invitations — with each row showing name/
   photo, age, gender, skill level, labels, flags, attendance rate, and last
   activity; the trainer can sort and page through the list, and click into
   any player's detail view.
   Source: US-03.01 "Trainer Views Player List" (line 129)

2. **Trainer** searches their player list so that they can quickly find a
   specific player.
   Path: Trainer types into a search bar scoped to the CRM tool (not
   global); results update in real time by player name, parent name/email,
   or team/school/club, matching partial strings; a clear (X) button resets
   the search, and an empty result shows "No players found. Try different
   search terms."
   Source: US-03.02 "Trainer Searches Players" (line 159)

3. **Trainer** creates and applies custom labels so that they can organize
   players into meaningful groups.
   Path: Trainer creates a label (name up to 50 characters, a color) from
   "Manage Labels"/"+ New Label"; from a player's detail view they add one
   or more labels via a multi-select dropdown, which then show as colored
   badges; the trainer can later edit a label's name/color or delete it
   (removing it from every player, with a confirmation showing the affected
   player count).
   Source: US-03.03 "Trainer Creates and Applies Labels" (line 182)

4. **Trainer** applies system-defined flags to a player so that they can
   track important player statuses.
   Path: From a player's detail view the trainer adds one of the 8 system
   flags, optionally with an explanatory note; the flag appears as a badge
   with a timestamp and who applied it; flags can later be removed with a
   "Mark as resolved?" confirmation, which hides them from the active view
   while preserving their history in an audit log.
   Source: US-03.04 "Trainer Applies System Flags" (line 219)

5. **Trainer** adds notes to a player so that they can track important
   information and context.
   Path: Trainer adds a general note (up to 1000 characters) from the
   player detail's Notes section, or a note tied to a specific event from
   that event's row in Event History; notes display chronologically with
   author and timestamp; the trainer can edit their own note within 24
   hours and delete it any time, but cannot edit a coach's note.
   Source: US-03.05 "Trainer Adds Notes to Player" (line 260)

6. **Trainer** segments and filters players so that they can target
   specific groups for communications or events.
   Path: Trainer opens the filter panel and combines multi-select criteria
   — skill level, age, gender, labels, flags, team/school/club, attendance
   history (attended-count, attended-in-last-Y-days, attendance-rate,
   no-shows), registration date range, and last-activity band — all
   combined with AND logic; the list updates with a match count, removable
   filter badges, and a clear-all option; zero matches shows "No players
   match your filters. Try adjusting criteria."
   Source: US-03.06 "Trainer Segments and Filters Players" (line 295)

7. **Trainer** views a player's full detail so that they can understand
   that player's history and status.
   Path: Trainer clicks a player to open a detail view with a basic profile
   (contact info, skill level, parent info if a child, Best Times/
   Availability from Epic-01), a labels section, a flags section (with
   who/when/note), a chronological notes section, an event history (date,
   event, attendance status, per-event notes, attendance rate, no-show
   count, last event), and an optional coach-feedback section that is
   read-only to the trainer.
   Source: US-03.07 "Trainer Views Player Detail" (line 330)

8. **Trainer** views the Quick View dashboard so that they can understand
   their business performance at a glance.
   Path: Trainer opens "Dashboard"/"Quick View" and sees events this week
   (vs. last week), RSVPs by event type, the Top 10 Players ranked by
   90-day attendance, flag-type counts (clickable to filter the player
   list), ShareLink opens and new joins this week, and (optional MVP)
   attendance trends; the dashboard is calculated from the database and
   updates on refresh.
   Source: US-03.08 "Trainer Views Quick View Dashboard" (line 377)

9. **Coach** views the players assigned to their events so that they can
   prepare and provide feedback.
   Path: Coach opens "Players" (coach view) and sees only players who have
   RSVP'd to events the coach is assigned to — never the trainer's full
   roster; each row shows name, photo, skill level, and an attendance
   summary with this coach; clicking a player opens a coach-scoped detail
   limited to their shared session history, read-only flags/labels, and
   past feedback from this coach.
   Source: US-03.09 "Coach Views Assigned Players" (line 427)

10. **Coach** adds session feedback for a player so that trainers and
    parents can see player progress.
    Path: From the coach-view player detail, the coach selects a recent
    shared session and writes feedback (up to 500 characters); the feedback
    is visible to the trainer (in session history) and to the coach (their
    own feedback), with player/parent visibility marked optional MVP; the
    coach can edit or delete their own feedback within 24 hours.
    Source: US-03.10 "Coach Adds Session Feedback" (line 459)

11. **Coach** invites a player via ShareLink so that they can expand the
    trainer's player base.
    Path: Coach clicks "Invite Player" to generate a unique ShareLink,
    copies it or sends it by email; when the invited player clicks it, an
    unauthenticated player is redirected to log in and, once logged in, is
    associated with the trainer's organization and appears in the
    trainer's CRM; the coach can see the invitation's tracked opens and
    status but cannot apply labels or flags to players they invite.
    Source: US-03.11 "Coach Invites Player via ShareLink" (line 494)

12. **Super Admin** views the CRM system-wide so that they can monitor
    platform activity and support trainers.
    Path: Super Admin opens "CRM Master"/"All Players" to see every player
    across every trainer, searches by player/trainer/email, filters by
    trainer/skill/age/gender/flags/registration date/last activity, and can
    apply flags across trainers from a player's detail view; a system-wide
    Quick View dashboard shows total players, sessions this week,
    system-wide Top Players, flag counts, and revenue, with drill-down by
    trainer.
    Source: US-03.12 "Super Admin Views System-Wide CRM" (line 518)

13. **Trainer** invites players via ShareLink so that they can grow their
    player base.
    Path: Trainer gets one reusable static mass-invite link (shareable via
    email/SMS/social/website embed); when a player clicks it, an
    unauthenticated player is redirected to log in/sign up and, once
    logged in, is associated with the trainer and appears in their CRM;
    optionally the trainer can also generate unique one-time links per
    player/parent; all link opens and successful joins are tracked and
    shown in the Quick View dashboard.
    Source: US-03.13 "Trainer Invites Players via ShareLink" (line 556)

## Acceptance criteria

Every checkbox below is renumbered sequentially as AC-03-1 .. AC-03-70 from
the epic's per-story "Acceptance Criteria" lists (US-03.01–US-03.13), three
criteria derived from "In Scope (MVP)" scope notes with no dedicated story
(Event-Scoped Player Access and its search limitation, both under "Player
Profiles (Coach View)"; Coach Hours Tracking, under "Quick View Dashboard"),
and the epic-level "Acceptance Criteria (Epic-Level)" section. Where an
epic-level or scope-note criterion restates a story criterion, the story
version is kept and both origins are cited; where an epic-level criterion
has no matching story text anywhere, it is kept as its own criterion cited
to the epic-level section alone.

**US-03.01 — Trainer Views Player List** (line 129)
- [ ] **AC-03-1** Trainer can navigate to "CRM Tools"/"Players" to view a list of all players associated with their organization — `Epic-03_CRM_Player_Management_SPEC.md` § "US-03.01: Trainer Views Player List" (lines 136-137; also epic-level AC § "Acceptance Criteria (Epic-Level)", line 931)
- [ ] **AC-03-2** The player list is populated from three association sources: ShareLink invitations, event registrations (RSVPs), and coach invitations — § "US-03.01..." (lines 138-141; also Business Rules "Player Association Rules", line 671)
- [ ] **AC-03-3** Each player row displays name and photo/avatar, age, gender, skill level, labels, flags, attendance rate (% attended), and last activity date — § "US-03.01..." (lines 142-149)
- [ ] **AC-03-4** Clicking a player row opens the player detail view — § "US-03.01..." (line 150)
- [ ] **AC-03-5** The player list is paginated at 50 players per page — § "US-03.01..." (line 151)
- [ ] **AC-03-6** The list can be sorted by Name (A-Z or Z-A), Last Activity, or Attendance Rate — § "US-03.01..." (line 152)
- [ ] **AC-03-7** The player list loads in under 2 seconds for 500 players — § "US-03.01... — Performance" (line 155; also "Performance & Scale Targets", line 905; epic-level AC § Performance, line 970)

**US-03.02 — Trainer Searches Players** (line 159)
- [ ] **AC-03-8** A search bar at the top of the player list lets the trainer search by player name (first, last, or full), parent name or email (for child accounts), or team/school/club (if present); results update in real time as the trainer types, with a clear (X) button to reset the search — § "US-03.02: Trainer Searches Players" (lines 166-173; also epic-level AC, line 932, for the name/email/team/school portion)
- [ ] **AC-03-9** When no player matches the search, the system shows "No players found. Try different search terms." — § "US-03.02..." (line 174)
- [ ] **AC-03-10** Search is scoped to the CRM tool only, not a global search, and matches partial strings (e.g. "John" finds "Johnny") — § "US-03.02... — Tool-Specific Search" (lines 177-178)

**US-03.03 — Trainer Creates and Applies Labels** (line 182)
- [ ] **AC-03-11** Trainer can create a custom label from CRM Tools via "Manage Labels" or "+ New Label", entering a required label name (max 50 characters) and a color from a color picker with presets; saving creates the label and it appears in the label list — § "US-03.03... — Acceptance Criteria - Create Label" (lines 189-195; also epic-level AC, line 933)
- [ ] **AC-03-12** From the player detail view, the trainer clicks "+ Add Label", selects one or more labels (multi-select) from a dropdown of all created labels, and saves to apply them; applied labels display as colored badges in both the player detail and list views, and a player can have multiple labels — § "US-03.03... — Acceptance Criteria - Apply Label" (lines 198-203; also epic-level AC § Data Integrity, line 963)
- [ ] **AC-03-13** Trainer can edit a label's name or color, and delete a label (which removes it from all players and requires confirmation) — § "US-03.03... — Acceptance Criteria - Manage Labels" (lines 206-207)
- [ ] **AC-03-14** Each label shows its usage count, e.g. "Applied to 12 players" — § "US-03.03..." (line 208)

**US-03.04 — Trainer Applies System Flags** (line 219)
- [ ] **AC-03-15** From the player detail view, the trainer clicks "+ Add Flag" and selects from the 8 system-defined flags (see Data requirements for the full list and definitions), optionally adding a note explaining the flag; saving applies the flag as a badge/icon in the player detail and list views, recording the timestamp and who applied it — § "US-03.04... — Acceptance Criteria - Apply Flag" (lines 226-240; also Business Rules "Flag Rules", lines 699-712; epic-level AC, line 934)
- [ ] **AC-03-16** The player list shows flag icons as small badges; the player detail view shows the full flag list with notes; and a per-player flag count is shown, e.g. "3 flags" — § "US-03.04... — Acceptance Criteria - View Flags" (lines 243-245)
- [ ] **AC-03-17** Clicking a flag offers a "Remove Flag" option with a "Mark as resolved?" confirmation; on confirmation the flag is removed from the active-flags view but its history is preserved in an audit log — § "US-03.04... — Acceptance Criteria - Remove Flag" (lines 248-251; also Business Rules "Flag Rules — Resolution", lines 715-717; epic-level AC § Data Integrity, line 964)
- [ ] **AC-03-18** Flag permissions differ by role: Trainer can apply, view, and remove all flags for their own players; Coach can apply and view flags for assigned players (the source marks this "optional MVP"); Super Admin can apply, view, and remove flags across all trainers — § "US-03.04... — Permissions" (lines 254-256). See Open questions: this appears to conflict with Q-03.05, which asks whether coaches can apply flags at all (analyst-raised).

**US-03.05 — Trainer Adds Notes to Player** (line 260)
- [ ] **AC-03-19** From the player detail view's "Notes" section, the trainer clicks "+ Add Note", enters text (up to 1000 characters), and saves; the note appears in a chronological list (most recent first) showing the text, who added it, and the timestamp — § "US-03.05... — Acceptance Criteria - General Notes" (lines 267-272; also epic-level AC, line 935)
- [ ] **AC-03-20** From the player detail view's "Event History" section, each event row has its own "+ Add Note" button that opens a text area tied to that specific event; saving adds the note to the event and it becomes visible in the player's history as "[Event Title] - [Date]: [Note]" — § "US-03.05... — Acceptance Criteria - Per-Event Notes" (lines 275-279)
- [ ] **AC-03-21** The trainer can edit their own notes within 24 hours of creation and delete their own notes at any time; the trainer cannot edit or delete coach notes (read-only to the trainer); Super Admin can edit or delete any notes — § "US-03.05... — Acceptance Criteria - Edit/Delete Notes" (lines 282-285; also Business Rules "Notes Rules — Edit Rules", lines 737-741; epic-level AC § Data Integrity, line 965)

**US-03.06 — Trainer Segments and Filters Players** (line 295)
- [ ] **AC-03-22** A filter panel (left side or top of the player list) lets the trainer combine multiple, multi-select filter criteria: Skill Level (Beginner, Intermediate, Advanced, Elite), Age (ranges 5-7, 8-10, 11-13, 14-16, 17+, or custom), Gender (All, Male, Female, Other), Labels (one or more), Flags (one or more), and Team/School/Club — § "US-03.06: Trainer Segments and Filters Players" (lines 302-309; also epic-level AC, line 936)
- [ ] **AC-03-23** Attendance History filters include: attended more than X events (number input), attended within the last Y days (number input), attendance rate greater than Z% (0-100% slider), and no-shows greater than N (number input) — § "US-03.06..." (lines 310-314; also Business Rules "Segmentation Rules — Attendance Filters", lines 750-754)
- [ ] **AC-03-24** Additional filters include Registration Date (date range picker) and Last Activity, with three bands: Active (within 30 days), Inactive (30-90 days), Churned (>90 days) — § "US-03.06..." (lines 315-316)
- [ ] **AC-03-25** Applying filters updates the player list to show only matching players, displays a match count (e.g. "15 players match your filters"), shows active filters as removable badges, and offers a "Clear all filters" button — § "US-03.06..." (lines 317-320)
- [ ] **AC-03-26** Filters combine with AND logic (a player must match all active criteria); if no players match, the system shows "No players match your filters. Try adjusting criteria." — Business Rules "Segmentation Rules — Filter Combination" and "— Empty Results" (lines 745-748, 756-757; also epic-level AC § Data Integrity, line 967)

**US-03.07 — Trainer Views Player Detail** (line 330)
- [ ] **AC-03-27** Clicking a player from the list opens the player detail page/modal, showing a Basic Profile section with name, photo, age, gender, skill level, email, phone, school/team/club, parent information (if a child account), and Best Times/Availability carried over from Epic-01 — § "US-03.07: Trainer Views Player Detail" (lines 337-344)
- [ ] **AC-03-28** The player detail view has a Labels section: all applied labels as colored badges, an "+ Add Label" button, and click-to-remove — § "US-03.07..." (lines 345-348)
- [ ] **AC-03-29** The player detail view has a Flags section: all applied flags as badges/icons with who applied them, when, and the note; an "+ Add Flag" button; and click to view details or remove — § "US-03.07..." (lines 349-353)
- [ ] **AC-03-30** The player detail view has a Notes section: a chronological list of general notes, each showing text, author, and timestamp, with an "+ Add Note" button — § "US-03.07..." (lines 354-357)
- [ ] **AC-03-31** The player detail view has an Event History section listing all events attended with this trainer, with columns for Date, Event Title, and Attendance Status (Present, Absent, Late, Excused), any per-event notes, an attendance rate (e.g. "18 of 20 events (90%)"), a no-show count (e.g. "2 no-shows"), and the date of the last event attended — § "US-03.07..." (lines 358-364)
- [ ] **AC-03-32** The player detail view has an optional-MVP Coach Feedback section showing notes left by coaches during events (coach name, date, event, feedback text), read-only to the trainer — § "US-03.07..." (lines 365-368)
- [ ] **AC-03-33** From the player detail view the trainer can edit the player profile's limited fields (skill level, notes), add a label/flag/note, and view the player's full event history, paginated if it exceeds 50 events — § "US-03.07... — Actions Available" (lines 370-373)

**US-03.08 — Trainer Views Quick View Dashboard** (line 377)
- [ ] **AC-03-34** Trainer can navigate to "Dashboard"/"Quick View" — § "US-03.08: Trainer Views Quick View Dashboard" (line 384)
- [ ] **AC-03-35** The dashboard shows an "Events This Week" count with a comparison to last week (e.g. "+3 vs last week") — § "US-03.08... — 1. Events This Week" (lines 387-389; also epic-level AC § Quick View Dashboard, line 939 — worded "sessions this week" there, see Open questions, analyst-raised)
- [ ] **AC-03-36** The dashboard shows total RSVPs this week, broken down by event type (Training, Private, Small Group) — § "US-03.08... — 2. RSVPs" (lines 391-393)
- [ ] **AC-03-37** The dashboard shows a Top 10 Players list, ranked by number of events attended in the last 90 days, displaying player name and attendance count (e.g. "John Smith - 18 events"); clicking a player opens their player detail — § "US-03.08... — 3. Top Players ✅ CRITICAL" (lines 395-399; also Business Rules "Top Players Algorithm", lines 759-780 — see there for the full, precise ranking definition; epic-level AC, line 940)
- [ ] **AC-03-38** The dashboard shows a count of flagged players broken down by each of the 8 flag types, and clicking a flag type filters the player list by that flag — § "US-03.08... — 4. Flag Alerts" (lines 401-411; also epic-level AC, line 941)
- [ ] **AC-03-39** The dashboard shows the count of ShareLink opens this week and the count of new players who joined this week via ShareLink — § "US-03.08... — 5. ShareLink Tracking" (lines 413-415; also epic-level AC § ShareLink System, lines 959-960)
- [ ] **AC-03-40** (Optional MVP) The dashboard shows attendance trends: average attendance rate this week vs. last week, and the no-show rate this week — § "US-03.08... — 6. Attendance Trends (optional MVP)" (lines 417-419)
- [ ] **AC-03-41** Dashboard data updates in real time or on page refresh, and all metrics are calculated from the database rather than entered manually — § "US-03.08... — Refresh" (lines 421-423)
- [ ] **AC-03-42** The Quick View dashboard loads in under 2 seconds — epic-level AC § Quick View Dashboard (line 942; also § Performance, line 972; "Performance & Scale Targets", line 909)

**US-03.09 — Coach Views Assigned Players** (line 427)
- [ ] **AC-03-43** Coach navigates to "Players" (Coach view) and sees only players who have RSVP'd to events where the coach is assigned — not the trainer's full player list; each row shows name/photo, skill level, and an attendance summary (e.g. "12 events with you") — § "US-03.09: Coach Views Assigned Players" (lines 434-440; also epic-level AC § Coach Experience, line 945)
- [ ] **AC-03-44** Clicking a player opens a coach-scoped player detail view showing basic info (skill level, age), attendance history limited to events with this coach (date, event, attendance status, and attendance rate with this coach), flags and labels (read-only), past feedback from this coach, and an "+ Add Event Feedback" button — § "US-03.09... — Player Detail (Coach View)" (lines 441, 444-450). Note: the source cites this feedback button as "(see US-03.11)", but the feedback story is actually US-03.10 — see Open questions (analyst-raised, confirmed defect).
- [ ] **AC-03-45** The coach cannot see players outside their assigned events, cannot edit player profiles except feedback (read-only), and cannot remove labels or flags (read-only) — § "US-03.09... — Limitations" (lines 453-455)

**US-03.10 — Coach Adds Session Feedback** (line 459)
- [ ] **AC-03-46** From the coach-view player detail, the coach clicks "+ Add Session Feedback", selects a session from a dropdown of their recent sessions with this player, enters feedback text (up to 500 characters), and saves — § "US-03.10: Coach Adds Session Feedback" (lines 466-470)
- [ ] **AC-03-47** Saved feedback is visible to the trainer (in the player's detail/session history) and to the coach (own feedback); visibility to the player/parent is marked "optional MVP - may defer to Phase 2" in the source — § "US-03.10..." (lines 471-474; also epic-level AC § Coach Experience, lines 946, 948). See Open questions: this bears on Q-03.01 and the Out-of-MVP-scope list (analyst-raised).
- [ ] **AC-03-48** In the trainer's view, feedback displays as "[Session Title] - [Date]: [Feedback]" linked to the specific session, with the coach's name shown, e.g. "Coach Mike: 'Great improvement on footwork.'" — § "US-03.10... — Feedback Display" (lines 477-479)
- [ ] **AC-03-49** Coach can edit or delete their own feedback within 24 hours; the trainer cannot edit coach feedback (read-only); Super Admin can edit or delete any feedback at any time — § "US-03.10... — Edit/Delete" (lines 482-485)

**US-03.11 — Coach Invites Player via ShareLink** (line 494)
- [ ] **AC-03-50** From the coach's player list, clicking "Invite Player" generates a unique ShareLink for this invitation (format `https://app.platform.com/invite/[unique-code]`), which the coach can copy or send via email — § "US-03.11: Coach Invites Player via ShareLink" (lines 501-504; also epic-level AC § Coach Experience, line 947; § ShareLink System, line 958)
- [ ] **AC-03-51** When the invited player clicks the link, an unauthenticated player is redirected to login; after login the player is associated with the trainer's organization and appears in the trainer's CRM — § "US-03.11..." (lines 505-508)
- [ ] **AC-03-52** The link's opens are tracked in metrics visible to the trainer, and the coach can view their sent invitations and status (pending, accepted) — § "US-03.11..." (lines 509-510; also epic-level AC § ShareLink System, line 959)
- [ ] **AC-03-53** The coach cannot apply labels or flags to players they invite (trainer-only), and can only view invited players once they appear in the coach's own assigned sessions — § "US-03.11... — Limitations" (lines 513-514)

**US-03.12 — Super Admin Views System-Wide CRM** (line 518)
- [ ] **AC-03-54** Super Admin can navigate to "CRM Master"/"All Players" to see a list of all players from all trainers, with tool-specific search by player name, trainer name, or email — § "US-03.12: Super Admin Views System-Wide CRM" (lines 525-527; also epic-level AC § Super Admin, lines 951-952)
- [ ] **AC-03-55** Super Admin can filter the system-wide player list by Trainer (dropdown/autocomplete), skill level, age, gender, flags, registration date range, and last activity — § "US-03.12..." (lines 528-533)
- [ ] **AC-03-56** Clicking a player opens a Super-Admin-scoped player detail; from there Super Admin can apply flags across trainers and view a player's history across trainers if the player is associated with more than one — § "US-03.12..." (lines 534-537; also epic-level AC § Super Admin, line 953)
- [ ] **AC-03-57** The Super Admin Quick View dashboard shows system-wide metrics: total players across all trainers, sessions this week (all trainers), system-wide Top Players (most active across the platform), system-wide flag counts, and system-wide revenue (linked to Stripe, referencing D-ARCH-001); Super Admin can drill down by selecting a trainer to view that trainer's specific metrics — § "US-03.12... — Quick View Dashboard (Super Admin)" (lines 539-547; also epic-level AC § Super Admin, line 954)
- [ ] **AC-03-58** Super Admin can view and edit all player data and can apply or remove flags across trainers, but cannot edit trainer-specific labels, respecting each trainer's own customization — § "US-03.12... — Permissions" (lines 550-552)

**US-03.13 — Trainer Invites Players via ShareLink** (line 556)
- [ ] **AC-03-59** From CRM Tools → "Invite Players", the trainer sees a single static, reusable mass invite link per trainer (format `https://app.platform.com/join/[trainer-unique-code]`), which can be copied, shared via email/SMS/social media, or embedded in a website — § "US-03.13... — Acceptance Criteria - Static Mass Invite Link" (lines 563-569; also epic-level AC § ShareLink System, line 957)
- [ ] **AC-03-60** When a player clicks the trainer's link, an unauthenticated player is redirected to login/signup; after login the player's account is associated with the trainer's organization and the player appears in the trainer's CRM — § "US-03.13..." (lines 570-573)
- [ ] **AC-03-61** (Optional MVP) Trainer can generate unique, one-time links per player/parent to track who invited whom and send personalized invitations; each link tracks who opened it, when, and whether it resulted in a join — § "US-03.13... — Acceptance Criteria - Unique Invite Links (Optional MVP)" (lines 576-578)
- [ ] **AC-03-62** ShareLink opens (clicks) and successful joins (player associations) are tracked and shown in the Quick View dashboard, e.g. "15 link opens this week" and "5 new players joined via ShareLink" — § "US-03.13... — ShareLink Tracking" (lines 581-585; also Business Rules "ShareLink Tracking Rules", lines 781-803; epic-level AC § ShareLink System, lines 959-960)
- [ ] **AC-03-63** (Optional MVP) A detailed tracking report shows date, link type (static or unique), opens, and joins — § "US-03.13... — ShareLink Tracking" (lines 586-587)

**In Scope (MVP) scope notes (not dedicated user stories)**
- [ ] **AC-03-64** Coach player access is strictly event-scoped: a coach assigned to zero events sees zero players; a coach assigned to Event A sees all players RSVP'd to Event A; a coach may click into a player's profile from an assigned event but cannot browse the trainer's full player list — `Epic-03_CRM_Player_Management_SPEC.md` § "In Scope (MVP) — Player Profiles (Coach View) — Event-Scoped Player Access" (lines 62-66; also US-03.09, lines 435-436, 453)
- [ ] **AC-03-65** Player search for a coach is limited to their assigned events; a coach cannot search across all of the trainer's players — § "In Scope (MVP) — Player Profiles (Coach View)" (line 71)
- [ ] **AC-03-66** The Quick View dashboard tracks total hours/sessions per coach, for trainer reference, displayed as e.g. "Coach has done 200 hours" or "covered 50 events"; this is analytics for trainers to manage coach payments that happen externally — the platform explicitly does NOT handle coach payments — § "In Scope (MVP) — Quick View Dashboard — Coach Hours Tracking" (lines 55-59). See Open questions: this feature has no dedicated story or data-requirements entry (analyst-raised).

**Epic-level (from "Acceptance Criteria (Epic-Level)", line 926) — not already covered above**
- [ ] **AC-03-67** Trainers can also search players by attendance — § "Acceptance Criteria (Epic-Level)" (line 932) only; no story describes an attendance search field (US-03.02 covers name/parent/team/school only — attendance appears elsewhere only as a *filter*, under US-03.06). See Open questions (analyst-raised).
- [ ] **AC-03-68** Every player-trainer association records its source (ShareLink, event registration, or coach invitation) and, if via ShareLink, which ShareLink — Data requirements "For Player-Trainer Associations" (lines 651-656; also Business Rules "Player Association Rules", lines 669-674; epic-level AC § Data Integrity, line 966)
- [ ] **AC-03-69** Search results return in under 500ms — epic-level AC § Performance (line 971; also "Performance & Scale Targets", line 907)
- [ ] **AC-03-70** (Process gate, not product behavior) Epic-03 is considered complete only once the demo is approved, all P0 and P1 open questions are resolved, the label and flag systems are validated, and the Top Players algorithm is confirmed — § "Acceptance Criteria (Epic-Level) — Approval" (lines 975-978)

**Final count: 70 acceptance criteria (AC-03-1 .. AC-03-70)**, covering all
13 user stories (US-03.01–US-03.13), three "In Scope (MVP)" scope-note
items with no dedicated story (event-scoped coach access, its search
limitation, and Coach Hours Tracking), and the epic-level completion
checklist, with most epic-level items folded into their matching story
criterion (both cited) and a few kept as their own criterion because no
story states them (attendance search, sub-500ms search performance, the
player-trainer association source rule, and the completion/approval gate).

## Business rules

Restated from "Business Rules & Logic" (line 667):

**Player Association**
- [ ] **BR-03-1** Players become associated with a trainer through exactly one of three sources: a ShareLink invitation (player clicks link, logs in, is associated), an event registration (player RSVPs to the trainer's event), or a coach invitation (coach invites the player, associating them with the trainer's organization) — `Epic-03_CRM_Player_Management_SPEC.md` § "Business Rules & Logic — Player Association Rules — How Players Get Associated with Trainer" (lines 671-674)
- [ ] **BR-03-2** A player can be associated with multiple trainers; each trainer sees only their own association with the player in their CRM; Super Admin sees all of a player's trainer associations — § "...Player Association Rules — Multi-Trainer Associations" (lines 676-679; also "Implementation Notes — Security Requirements", line 1067, "Prevent cross-trainer data access")

**Label**
- [ ] **BR-03-3** A trainer can create an unlimited number of labels; label names must be unique within the trainer's own organization and are case-insensitive (e.g. "Elite" and "elite" are the same label) — § "...Label Rules — Creation" (lines 684-686)
- [ ] **BR-03-4** A player can have multiple labels; labels are trainer-specific, so the same label name created by two different trainers (e.g. both named "Elite") are distinct labels; removing a label from a player does not delete the player — § "...Label Rules — Application" (lines 688-691)
- [ ] **BR-03-5** Deleting a label removes it from every player it was applied to, and requires confirmation of the form "Remove [Label] from [N] players?" — § "...Label Rules — Deletion" (lines 693-695)

**Flag**
- [ ] **BR-03-6** The platform defines exactly 8 system flags and no custom flags can be added: (1) Behavior, (2) High no-show rate, (3) Injured, (4) Medical restriction, (5) Scholarship, (6) Financial aid, (7) Contact priority, (8) Attendance risk — § "...Flag Rules — System-Defined Flags" (lines 699-707; also US-03.04 "Acceptance Criteria - Apply Flag", lines 227-235, which pairs each name with a short definition — see Data requirements)
- [ ] **BR-03-7** A player can have multiple active flags; a flag's explanatory note is optional though recommended; every flag application is logged with who applied it, when, and the note — § "...Flag Rules — Application" (lines 709-712)
- [ ] **BR-03-8** A flag can be marked "Resolved"; resolved flags are hidden from the active view but kept in history; a trainer can reapply the same flag later if the issue recurs — § "...Flag Rules — Resolution" (lines 714-717)
- [ ] **BR-03-9** (Future, not MVP) Auto-flagging is planned but deferred to Phase 2: "High no-show rate" would auto-apply if a player no-shows more than 3 times in 30 days, and "Attendance risk" would auto-apply if a player has no activity in 60 days — § "...Flag Rules — Auto-Flagging (Future)" (lines 719-721; also "Out of Scope (Post-MVP)", line 87). See Open questions: Q-03.04 asks the same MVP-vs-Phase-2 question this rule already answers (analyst-raised).

**Notes**
- [ ] **BR-03-10** General notes are added by the trainer and visible to the trainer and coaches; they are not tied to a specific session, display chronologically (most recent first), and have no hard MVP character limit though a reasonable limit of 1000 characters is suggested — § "...Notes Rules — General Notes" (lines 725-729)
- [ ] **BR-03-11** Per-session notes are added by a coach during or after a session, tied to that specific session in the player's history; the trainer can view them read-only; Player/Parent visibility is marked "optional MVP, may defer to Phase 2" — § "...Notes Rules — Per-Session Notes" (lines 731-735)
- [ ] **BR-03-12** A note's creator can edit it within 24 hours of creation; after 24 hours it becomes read-only (editing requires contacting Super Admin); a trainer cannot edit coach notes (read-only to the trainer); Super Admin can edit any note at any time — § "...Notes Rules — Edit Rules" (lines 737-741)

**Segmentation**
- [ ] **BR-03-13** All active filters combine with AND logic — e.g. "Skill: Beginner" AND "Attendance Rate >80%" AND "Label: Elite Squad" returns only players matching all three — § "...Segmentation Rules — Filter Combination" (lines 745-748)
- [ ] **BR-03-14** Attendance filters are defined precisely as: "Attended > X sessions" = total sessions with the trainer; "Attended in last Y days" = sessions within that date range; "Attendance rate > Z%" = percentage of registered sessions attended; "No-shows > N" = count of Absent-status sessions — § "...Segmentation Rules — Attendance Filters" (lines 750-754)
- [ ] **BR-03-15** When no players match the active filters, the system shows "No players match your filters. Try adjusting criteria." — § "...Segmentation Rules — Empty Results" (lines 756-757)

**Top Players Algorithm**
- [ ] **BR-03-16** The Top Players ranking (marked "✅ CONFIRMED" in the source) counts each player's sessions attended in the last 90 days from today; the highest attendance count ranks #1; ties are broken alphabetically by player name — § "...Top Players Algorithm — Ranking Logic" (lines 761-764)
- [ ] **BR-03-17** Only two attendance statuses count toward the Top Players total — Present and Late (Late is explicitly marked "attended") — Absent and Excused do NOT count — § "...Top Players Algorithm — Attendance Status Counted" (lines 766-770)
- [ ] **BR-03-18** The dashboard displays the Top 10 players, each shown as player name plus session count (e.g. "John Smith - 18 sessions"); clicking a player opens their player detail — § "...Top Players Algorithm — Display" (lines 772-775)
- [ ] **BR-03-19** If fewer than 10 players qualify, all qualifying players are shown; a player with 0 sessions in the last 90 days is not shown in Top Players at all — § "...Top Players Algorithm — Edge Cases" (lines 777-779)

**ShareLink Tracking**
- [ ] **BR-03-20** The static mass invite link is one per trainer, reusable with unlimited uses; the system tracks its total opens (clicks) and successful joins (associations created) — § "...ShareLink Tracking Rules — Static Mass Invite Link" (lines 783-787)
- [ ] **BR-03-21** (Optional MVP) Unique invite links are generated per invitation and may be one-time or multi-use at the trainer's preference; opens are tracked per link, along with who joined via which link — § "...ShareLink Tracking Rules — Unique Invite Links" (lines 789-793)
- [ ] **BR-03-22** Each link click is logged (timestamp, IP optional) and counted per link, and displayed in the Quick View dashboard — § "...ShareLink Tracking Rules — Opening Tracking" (lines 795-798)
- [ ] **BR-03-23** When a player associates with a trainer via a link, the link is marked "Joined"; the joined count is displayed in Quick View — § "...ShareLink Tracking Rules — Join Tracking" (lines 800-802)

## Data requirements

Restated from "Data Requirements" (line 591) — entities, the fields the
epic names, and the relationships it names. No schema, keys, or types are
proposed here.

**Note on Player Profile** (line 595): the epic marks this entity as
extending the Player profile Epic-01 already defines, rather than
introducing a new one; this spec lists only what Epic-03 adds on top of
Epic-01's fields.

- **Player Profile** (extends Epic-01's Player profile): player unique identifier, User account reference (parent account if the player is a child), trainer association (which trainer's organization), skill level (updated by trainer), school/team/club, registration date (when associated with this trainer), last activity timestamp, active/inactive status. Relationship: extends a Player from Epic-01; associated with a Trainer. (line 595)
- **Label**: label unique identifier, trainer who created it, label name (up to 50 characters), color code (hex value), created timestamp. Relationship: created by a Trainer. (line 605)
- **Player-Label Association**: which player, which label, applied by (trainer), applied timestamp. Relationship: join entity between Player and Label. (line 612)
- **Flag**: flag type — one of the **8 system-defined flags**: (1) Behavior (behavioral issues), (2) High no-show rate (attendance concern), (3) Injured (medical - cannot participate), (4) Medical restriction (can participate with restrictions), (5) Scholarship (financial aid recipient), (6) Financial aid (payment assistance), (7) Contact priority (requires special attention), (8) Attendance risk (at risk of dropping out); applied to which player; applied by (trainer or coach); applied timestamp; optional note (why the flag was applied); status (active, resolved); resolved by (if resolved); resolved timestamp. Relationship: applied to a Player by a User (Trainer, Coach, or Super Admin). (line 618; flag names and definitions from US-03.04, lines 227-235)
- **Note**: note unique identifier, which player, note type (general or session-specific), which session (if session-specific), note text (up to 1000 characters), created by (trainer or coach), created timestamp, last edited timestamp, edited by. Relationship: belongs to a Player; authored by a User. (line 628)
- **ShareLink Invitation**: link unique code, link type (static mass or unique), created by (trainer or coach), created for (which trainer's organization), intended recipient email or name (if unique), created timestamp, opens count, last opened timestamp, joined status (boolean), joined timestamp. Relationship: belongs to a Trainer; created by a Trainer or Coach. (line 639)
- **Player-Trainer Association**: which player, which trainer, association source (sharelink, event_registration, or coach_invite), which ShareLink (if the source is sharelink), associated timestamp. Relationship: join entity between Player and Trainer. (line 651)
- **Quick View Metrics** (calculated, not stored as its own record): Top Players ranking (queried from last-90-days attendance), flag counts (queried from active flags), session counts (queried for the current week), RSVP counts (queried for the current week), ShareLink tracking (queried from link opens). (line 658)

## Edge cases

| Case | Expected |
|---|---|
| Player has 0 sessions attended in the last 90 days | Not shown in Top Players at all — Business Rules "Top Players Algorithm — Edge Cases" (line 779); Testing Considerations (line 1020) |
| Fewer than 10 players qualify for Top Players | All qualifying players are shown — Business Rules "Top Players Algorithm — Edge Cases" (line 778) |
| Player has multiple labels and multiple flags | Both display concurrently as badges/icons on the player row and in player detail (no stated limit on either) — US-03.03 (line 203); Business Rules "Flag Rules — Application" (line 710); Testing Considerations (line 1021) |
| Filter combination matches 0 players | "No players match your filters. Try adjusting criteria." — Business Rules "Segmentation Rules — Empty Results" (lines 756-757); Testing Considerations (line 1022) |
| Player search matches 0 players | "No players found. Try different search terms." — US-03.02 (line 174) |
| A label is deleted | Removed from all players it was applied to; confirmation required ("Remove [Label] from [N] players?") — Business Rules "Label Rules — Deletion" (lines 693-695); Testing Considerations (line 1024) |
| A flag is resolved | Hidden from the active view but kept in flag history; can be reapplied later if the issue recurs — Business Rules "Flag Rules — Resolution" (lines 714-717); Testing Considerations (line 1025) |
| A note is edited after its 24-hour edit window | Edit blocked; note becomes read-only (contact Super Admin to edit) — Business Rules "Notes Rules — Edit Rules" (lines 738-739); Testing Considerations (line 1026) |
| A ShareLink is opened multiple times | Each open increments the link's tracked open count — Business Rules "ShareLink Tracking Rules — Opening Tracking" (lines 795-797); Testing Considerations (line 1027) |

Boundary/failure behavior the epic raises but does **not** state an
expected outcome for — what a coach should see when viewing a player with
no shared session history with that coach, Testing Considerations (line
1023) — is listed in Open questions instead of guessed here.

## Out of MVP scope

Verbatim from "Out of Scope (Post-MVP)" (line 81):

- Advanced analytics and reporting (charts, drill-downs)
- Player data export (CSV, PDF, Excel)
- Saved segments (bookmarked filter combinations)
- Automated flag application (e.g., auto-flag high no-show)
- Player messaging (in-app direct messages)
- Parent/player view of coach feedback (read-only display)
- Build a Bag (player-selected skills to work on)
- Bulk operations (bulk label, bulk flag, bulk export)
- Coach feedback ratings (star ratings, structured feedback)
- Player comparison (side-by-side player metrics)
- Revenue attribution per player (lifetime value)
- Advanced Top Players algorithm (weighted by event type, recency)
- Regional breakdowns (if multi-region)

## Open questions

### From the epic's "Questions / Open Issues" section (line 915, verbatim)

| ID | Question | Priority | Status | Owner |
|:---|:---|:---:|:---|:---|
| Q-03.01 | Can players/parents view coach feedback on their profiles? (Read-only display) | P2 | Open | Client |
| Q-03.04 | Auto-flag logic: "High no-show" if >3 no-shows in 30 days? MVP or Phase 2? | P2 | Open | Team |
| Q-03.05 | Can coaches apply flags to assigned players, or only trainers? | P2 | Open | Client |
| Q-03.07 | Label usage analytics: "This label is on 12 players, 3 haven't attended in 60 days"? | P2 | Open | Team |

### Analyst-raised

- **(analyst-raised)** ID gaps: the open-questions table's ID sequence jumps
  Q-03.01 → Q-03.04 → Q-03.05 → Q-03.07 (lines 919-922), skipping Q-03.02,
  Q-03.03, and Q-03.06 entirely. No trace of those three IDs appears
  anywhere else in the epic. Unclear whether questions were removed,
  renumbered, or never filled in.
- **(analyst-raised)** Possible contradiction: Q-03.01 asks whether
  players/parents can view coach feedback, but two other places in the same
  epic already assert an answer — US-03.10 marks player/parent visibility
  "optional MVP - may defer to Phase 2" (line 474), and "Out of Scope
  (Post-MVP)" explicitly excludes "Parent/player view of coach feedback
  (read-only display)" from MVP (line 89). Unclear whether the question is
  genuinely still open or the epic has already (inconsistently) answered it.
- **(analyst-raised)** Possible contradiction: Q-03.04 asks whether
  "High no-show" auto-flagging is MVP or Phase 2, but Business Rules "Flag
  Rules — Auto-Flagging (Future)" already states both auto-flag types
  "defer to Phase 2" (lines 719-721), and "Out of Scope (Post-MVP)" already
  excludes "Automated flag application" from MVP (line 87). Same pattern as
  above — the question reads open while the epic states an answer elsewhere.
- **(analyst-raised)** Possible contradiction: Q-03.05 asks whether coaches
  can apply flags to assigned players, but US-03.04 "Permissions" already
  states "Coach: Can apply, view flags for assigned players (optional
  MVP)" (line 255) as if decided. Unclear whether "optional MVP" itself is
  the open part, or whether the whole coach-flag capability is undecided.
- **(analyst-raised, confirmed defect)** US-03.09's coach-view player
  detail cites its own "+ Add Event Feedback" button as "(see US-03.11)"
  (line 450), but the session-feedback story is actually US-03.10 "Coach
  Adds Session Feedback" (line 459); US-03.11 is "Coach Invites Player via
  ShareLink" (line 494), an unrelated story.
- **(analyst-raised, methodology note)** The epic's top-level section
  numbering skips §14: it jumps from §13 "Acceptance Criteria (Epic-Level)"
  (line 926) to §15 "Mockups / Design References" (line 982). Cosmetic, but
  it is why every back-reference in this spec cites heading text plus line
  number rather than a bare section number.
- **(analyst-raised)** Terminology inconsistency, Event vs. Session: the
  epic uses "event" and "session" for what appears to be the same
  attendance unit without ever stating whether they are synonyms.
  US-03.08's dashboard AC labels its first metric "Events This Week" and
  its Top Players example "18 events" (lines 387, 398), while Business
  Rules "Top Players Algorithm" — the section marked "✅ CONFIRMED" — counts
  "sessions attended" and shows "18 sessions" (lines 762, 774), and User
  Flow 4 labels the same dashboard metric "Sessions This Week" (line 862).
  US-03.10 separately uses "session" for one coach's individual coaching
  occurrence with a player. Whether Event and Session denote the same
  entity, or Session is a sub-unit of an Event, is not stated.
- **(analyst-raised)** Cross-epic overlap, ShareLink ownership: US-03.13's
  static mass invite link (creation, link format, sharing channels)
  restates ShareLink mechanics Epic-01 already specifies in full (Epic-01's
  AC-01-73 and BR-01-14: a static, unlimited-use, no-expiry, trainer-issued
  link for players). Epic-03 additionally specifies a CRM-facing tracking
  layer (opens, joins, Quick View display) on top of what appears to be the
  same link type, and separately names Epic-06 (Marketing) as a further
  consumer of "ShareLinks tracked in CRM" ("Dependencies — Blocks These
  Epics", line 108). Three epics (01, 03, 06) touch the same ShareLink
  data without this epic stating which one owns creation vs. tracking vs.
  marketing analytics.
- **(analyst-raised)** Cross-epic gap, new ShareLink type: US-03.11 "Coach
  Invites Player via ShareLink" has the coach — not the trainer — generate
  an invite link directly to a player. Epic-01's ShareLink rules (BR-01-14,
  BR-01-15) only define trainer-issued links: a static player link and a
  unique coach link. Whether the coach's player-invite link in Epic-03 is a
  third link type, or reuses one of Epic-01's two types under a different
  issuer, is not stated in either epic file.
- **(analyst-raised)** `Epic_Areas_Plan.md` disagreement, export: the
  plan's Epic-03 overview says the CRM lets trainers "segment audiences,
  analyze engagement, and export data" (`Epic_Areas_Plan.md`, line 146),
  but this epic file places "Player data export (CSV, PDF, Excel)"
  explicitly in Out of Scope (Post-MVP) (line 85). Per this task's rule
  that the epic file wins, export is out of MVP scope for Epic-03. (Epic-08
  separately keeps camp/evaluation participant export in scope —
  `Epic-08_Forms_Registration_SPEC.md`, e.g. lines 66, 73, 255 — but that is
  different data from Epic-03's player/CRM records and not a conflict with
  this exclusion.)
- **(analyst-raised)** `Epic_Areas_Plan.md` disagreement, unlisted
  features: the plan's Epic-03 "Key Features" also lists "Player
  communication logs" (`Epic_Areas_Plan.md`, line 159) and "grade
  distribution" as part of "Core analytics" (line 173) — neither term
  appears anywhere in this epic file. Unclear whether these were dropped
  from scope when the epic file was written, or are gaps in the epic file.
- **(analyst-raised)** Unresolved edge case: Testing Considerations names
  "Coach views player with no session history with coach" (line 1023) as a
  case to test, but no expected outcome is stated anywhere in the epic for
  what the coach should see in that situation (empty state message,
  blocked access, or something else).
- **(analyst-raised)** No dedicated criteria: "Coach Hours Tracking"
  appears only once, as a bolded bullet under "In Scope (MVP) — Quick View
  Dashboard" (lines 55-59: track total hours/sessions per coach, show
  "Coach has done 200 hours" or "covered 50 events", analytics for trainers
  to manage external coach payments, platform does not handle coach
  payments). It has no user story, no Data Requirements entry, and no line
  in "Acceptance Criteria (Epic-Level)".
- **(analyst-raised)** No dedicated criteria: "Acceptance Criteria
  (Epic-Level)" states a trainer can "search players by name, email, team,
  school, attendance" (line 932), but US-03.02 — the story that defines the
  search fields — lists only player name, parent name/email, and
  team/school/club (lines 167-170); attendance is not among them.
  Attendance appears elsewhere only as a *filter* (US-03.06), not as a
  search field. Unclear whether "attendance" in the epic-level line is a
  drafting error or a search capability that was never detailed.
