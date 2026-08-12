# Spec: Epic-02 — Event Management & Scheduling

Source: `Epic-02_Event_Management_Scheduling_SPEC.md` (read-only client material)

> Citation note: as in the Epic-01 spec, every back-reference below cites
> heading **text** plus a line number, never a bare section number.
> Epic-02's own numbered sections (§1–§16) are not reused here, but several
> of its rule/criteria groups are organized under **bold sub-labels**
> (e.g., "Validation", "Required Fields", "Option A - Confirm") rather than
> markdown headings. Those are cited as a second `§` level —
> `§ "Parent Heading" § "Sub-label"` — the same convention the Epic-01 spec
> used for bold sub-labels like "Child CAN Do".

## Problem

Epic-02 implements the platform's core event creation, scheduling, and
attendance-tracking system, enabling trainers to create and manage training
events, players/parents to discover and register for them, and coaches to
view assignments and track attendance ("Description", line 7). Every
subsequent revenue- or engagement-facing epic depends on it: Epic-03's CRM
uses event attendance data, Epic-04 can assign content to events, Epic-05's
paid events require the event system to exist, and Epic-06's ShareLinks can
target specific events ("Dependencies — Blocks These Epics", lines
136-140). Without it, trainers cannot schedule or monetize training events,
players cannot discover or register for training, coaches cannot see their
assignments, the platform earns no revenue from paid events, and there is no
attendance data for analytics or accountability ("Description — Business
Value", lines 11-16); this epic exists to enable the complete event
lifecycle from creation to completion ("Description", line 18). Sports
trainers currently rely on fragmented tools — spreadsheets, group texts,
manual payment tracking — leaving players unable to easily discover events,
trainers losing revenue to poor attendance, and coaches without clear
visibility into their assignments ("Business Value — Problem Statement",
line 24). This epic therefore gives trainers full control over creating,
pricing, staffing, editing, and canceling their own events; lets
players/parents discover eligible events — including invitation-only ones —
and RSVP with either money or tokens; lets coaches confirm assignments and
record attendance; and lets Super Admin view and override events across all
trainers ("User Roles Involved", lines 152-157). Success is measured by
onboarding speed, RSVP friction, availability-matched bookings, coach
confirmation rate, attendance completion, event fill rate, and
private-event adoption ("Business Value — Success Metrics", lines 27-34),
and the whole epic depends on Epic-01 already providing the trainers,
coaches, and players who use it ("Dependencies — Required Before This
Epic", line 134).

## User scenarios

1. **Trainer** creates a training event so that players can discover and
   register for it.
   Path: Trainer opens Event Builder, clicks "Create Event," fills required
   fields (title, event type, date/time, location, capacity) and optional
   fields (description, eligibility, pricing, coach), and saves; the event
   appears in the trainer's Event Builder list and in eligible players'
   Training Calendar.
   Source: US-02.01 "Trainer Creates Training Event" (line 163)

2. **Trainer** creates a private, invite-only event so that they can offer
   exclusive sessions to select players.
   Path: Trainer creates an event as usual, sets Visibility to "Private /
   Invite Only," and selects individual invitees from a multi-select player
   list; the event then appears only in invited players' calendars, and
   non-invited players cannot see it or reach it via a direct link.
   Source: US-02.02 "Trainer Creates Private Event" (line 196)

3. **Trainer** assigns a coach to an event, with an availability check, so
   that the event has an instructor.
   Path: Trainer selects a coach (themselves or any coach they've added)
   from a dropdown; if the time conflicts with the coach's stated
   availability, the trainer sees a warning and must enter a reason to
   override; the coach is then notified and the event appears in the
   coach's "Events to Confirm."
   Source: US-02.03 "Trainer Assigns Coach with Availability Check" (line
   221)

4. **Trainer** sees which players are available at a candidate event time
   so that they can schedule at times convenient for the most players.
   Path: While picking a date/time in Event Builder, the trainer sees a
   count of eligible players available at that time (from Epic-01
   availability data), and in the RSVP list each player shows a
   green/gray/red availability indicator.
   Source: US-02.04 "Trainer Sees Player Availability When Scheduling"
   (line 246)

5. **Trainer** duplicates an existing event so that they can quickly create
   a similar one without starting from scratch.
   Path: From the Event Builder list, the trainer clicks "Duplicate" on an
   event; a modal opens pre-filled with all original details, the trainer
   must set a new date/time (other fields optional to change), and saving
   creates a new, empty event — the original is untouched and RSVPs are not
   copied.
   Source: US-02.05 "Trainer Duplicates Event" (line 266)

6. **Player or Parent** views the Training Calendar so that they can find
   events to attend.
   Path: Player/Parent opens Training Calendar (month/week/day views), sees
   all public events they're eligible for plus any private events they're
   invited to, searches/filters within the tool, and sees an
   availability-match badge on each event if they've set availability
   preferences.
   Source: US-02.06 "Player Views Training Calendar" (line 293)

7. **Player or Parent** RSVPs to an event so that they can attend and
   secure their spot.
   Path: Player views event details and clicks "RSVP" (free) or "Register &
   Pay" (paid, redirected to Epic-05 payment or token use); a child
   account's RSVP requires parent approval; on success the event moves to
   "My Reservations" with a confirmation message and email; if the event is
   full, RSVP is disabled with an "Event Full" message.
   Source: US-02.07 "Player RSVPs to Event" (line 318)

8. **Player or Parent** cancels an RSVP so that they can free up their spot
   if they can't attend.
   Path: From "My Reservations," the player clicks "Cancel RSVP" and
   confirms; for a paid event, a refund follows the 24-hour policy (full
   refund if ≥24 hours out, none if <24 hours out); a child's cancellation
   requires parent approval, the same as a purchase.
   Source: US-02.08 "Player Cancels RSVP" (line 358)

9. **Coach** confirms or declines an event assignment so that trainers know
   their availability.
   Path: Coach opens "My Activities," reviews pending assignments in
   "Events to Confirm," and either confirms (moving it to "Assigned
   Sessions," notifying the trainer) or declines with an optional reason
   (notifying the trainer to find a replacement); an unanswered assignment
   can auto-confirm after 48 hours if the trainer has enabled that
   preference.
   Source: US-02.10 "Coach Confirms Event Assignment" (line 388)

10. **Coach** marks player attendance so that attendance records are
    accurate.
    Path: After the event starts, the coach opens it from "My Activities,"
    clicks "Take Attendance," and marks each RSVP'd player Present
    (default), Absent, Late, or Excused; the coach can only edit same-day,
    after which their view locks (trainer and Super Admin can still edit
    anytime).
    Source: US-02.11 "Coach Tracks Attendance" (line 418)

11. **Trainer** views who has registered for their event so that they can
    prepare for it.
    Path: Trainer opens an event's "RSVP List" tab to see each player's
    name, age, skill level, RSVP time, and payment status, plus a
    registration count against capacity; the trainer can manually add a
    player if space allows, remove a player (with refund if paid), or
    export the list as CSV.
    Source: US-02.12 "Trainer Views RSVP List" (line 452)

12. **Trainer** cancels an event so that players know it won't happen.
    Path: Trainer selects the event, clicks "Cancel Event," confirms and
    enters a required cancellation reason; the event is marked "Canceled,"
    all registered players and the coach are notified, and refunds are
    processed automatically for paid events.
    Source: US-02.13 "Trainer Cancels Event" (line 474)

13. **Trainer** edits an event after creation so that they can correct
    mistakes or adapt to changes.
    Path: Trainer opens the event and clicks "Edit"; any non-past field is
    editable, and changing date/time, location, coach, or price triggers
    targeted notifications to affected players/coaches; decreasing
    capacity below the current RSVP count is blocked until resolved.
    Source: US-02.14 "Trainer Edits Event" (line 499)

14. **Super Admin** views all events across all trainers so that they can
    monitor platform activity and support trainers.
    Path: Super Admin opens "Event Master Tool," sees every trainer's
    events with tool-specific search and filters (trainer, location, date
    range, type, status) and pagination, and can edit or cancel any event,
    override scheduling conflicts without warnings, view any RSVP list, or
    export events to CSV.
    Source: US-02.15 "Super Admin Views All Events" (line 530)

**Note**: The epic's stories are numbered US-02.01 through US-02.15, but
US-02.09 does not exist anywhere in the source — the numbering jumps from
US-02.08 to US-02.10. This spec therefore covers 14 stories, not 15. See
Open questions (analyst-raised).

## Acceptance criteria

Every checkbox below is renumbered sequentially as AC-02-1 .. AC-02-70 from
the epic's per-story "Acceptance Criteria" lists (including their nested
"Validation", "Edit Rules", "Capacity Changes", and "Option A/B" groups), 8
scope-only criteria derived from "In Scope (MVP)" bullets that have no
dedicated user story (Dual Pricing, Flexible Token Pricing, Dual Payment
Options, Recurring Events - Bulk Auto-Creation, Subscription RSVP
Restrictions, multi-trainer calendar separation, and the "Pending Coach
Confirmation" assignment state), and the epic-level "Acceptance Criteria
(Epic-Level)" section. Where an epic-level criterion restates a story or
scope criterion, the more detailed version is kept and both origins are
cited. Illustrative "Use Cases" / "Coach Conflict Scenarios" bullets attached
to some stories are not restated as criteria — they are examples, not
stated requirements.

**US-02.01 — Trainer Creates Training Event** (line 163)
- [ ] **AC-02-1** Trainer creates a training event from Event Builder via "Create Event," filling required fields — title, event type (Training Session, Private Session, or Small Group), start and end date/time, location (from the trainer's locations), and capacity — and optional fields — description, eligibility (age range, skill level, gender), pricing (Free, Paid Money, Paid Tokens), and coach assignment — `Epic-02_Event_Management_Scheduling_SPEC.md` § "US-02.01: Trainer Creates Training Event" (lines 170-181; also epic-level AC § "Event Creation (Trainer)", line 914)
- [ ] **AC-02-2** Saving creates the event, shows "Event created successfully," and the event appears in the trainer's Event Builder list and in eligible players' Training Calendar — § "US-02.01..." (lines 182-185)
- [ ] **AC-02-3** Event creation validation: title required (max 100 characters), date/time cannot be in the past, end time must be after start time, capacity must be > 0, and — if the event is paid — a price/token amount is required — § "US-02.01..." § "Validation" (lines 188-192)

**US-02.02 — Trainer Creates Private Event** (line 196)
- [ ] **AC-02-4** Trainer creates a private event using the same flow as a public event (US-02.01), then sets Visibility to "Private / Invite Only" — § "US-02.02: Trainer Creates Private Event" (lines 203-204)
- [ ] **AC-02-5** Trainer selects invitees via individual player selection only (multi-select list from the trainer's players); group-based selection is not available in MVP — § "US-02.02..." (lines 205-207)
- [ ] **AC-02-6** Saving creates the private event, visible only in invited players' calendars — § "US-02.02..." (lines 208-209)
- [ ] **AC-02-7** Non-invited players do not see the private event even if otherwise eligible by age/skill, and cannot access it via a direct link, which shows "Access Denied" — § "US-02.02..." (lines 210-211; also epic-level AC § "Event Creation (Trainer)", line 915; § "Event Discovery (Player/Parent)", line 924)

**US-02.03 — Trainer Assigns Coach with Availability Check** (line 221)
- [ ] **AC-02-8** Trainer selects a coach from a dropdown when creating or editing an event; the dropdown includes the trainer themselves (trainers can assign themselves as coach) and all coaches the trainer has added (Epic-01) — § "US-02.03: Trainer Assigns Coach with Availability Check" (lines 228-230)
- [ ] **AC-02-9** If the selected event time conflicts with the coach's "My Times" availability, the system shows a warning ("Coach [Name] is not available at this time. Continue anyway?"), the trainer can override by entering a required reason, and the override is logged (who, when, event, coach, reason) — § "US-02.03..." (lines 231-234; also epic-level AC § "Event Creation (Trainer)", line 916)
- [ ] **AC-02-10** If there is no conflict, the coach is assigned directly — § "US-02.03..." (line 235)
- [ ] **AC-02-11** The assigned coach sees the event in "Events to Confirm" (My Activities) and receives a notification by email and in-app — § "US-02.03..." (lines 236-237)

**US-02.04 — Trainer Sees Player Availability When Scheduling** (line 246)
- [ ] **AC-02-12** When selecting an event date/time in Event Builder, the trainer sees an availability count of eligible players at that time (e.g., "15 of 20 eligible players available at this time"), using player availability data from Epic-01 — § "US-02.04: Trainer Sees Player Availability When Scheduling" (lines 253-255; also epic-level AC § "Event Creation (Trainer)", line 917)
- [ ] **AC-02-13** In the RSVP list, each player shows an availability indicator: "Available" (green) if the player marked this time as available, "Unknown" (gray) if the player hasn't set availability for this time, or "Busy" (red) if the player marked a conflicting time — § "US-02.04..." (lines 256-259)

**US-02.05 — Trainer Duplicates Event** (line 266)
- [ ] **AC-02-14** Trainer duplicates an event from the Event Builder list via "Duplicate," opening a modal pre-filled with all original event details (title, type, location, coach, capacity, pricing, eligibility) — § "US-02.05: Trainer Duplicates Event" (lines 273-276; also epic-level AC § "Event Creation (Trainer)", line 918)
- [ ] **AC-02-15** The trainer must change the date/time (required) and may optionally modify any other field before saving — § "US-02.05..." (lines 277-278)
- [ ] **AC-02-16** Saving creates a new event with a new identifier; the original event remains unchanged, and RSVPs are not copied — the new event starts with none — § "US-02.05..." (lines 279-281)
- [ ] **AC-02-17** The new duplicated event appears in the calendar for eligible players — § "US-02.05..." (line 282)

**US-02.06 — Player Views Training Calendar** (line 293)
- [ ] **AC-02-18** Player/Parent navigates to "Training Calendar" with month, week, and day view toggles — § "US-02.06: Player Views Training Calendar" (lines 300-301)
- [ ] **AC-02-19** The calendar displays all public events for which the player is eligible (age, skill, gender) and private events where the player is invited; it does not show events for which the player is ineligible or non-invited private events — § "US-02.06..." (lines 302-305; also epic-level AC § "Event Discovery (Player/Parent)", line 923)
- [ ] **AC-02-20** Each event in the calendar shows title, date/time, location, capacity (X/Y), and price; clicking an event opens an event details modal — § "US-02.06..." (lines 306-307)
- [ ] **AC-02-21** Player can search events by title, location, or date, and filter by date range, type, or location, scoped to the Training Calendar tool (not a global search) — § "US-02.06..." (lines 308-309, 314; also epic-level AC § "Event Discovery (Player/Parent)", line 925)
- [ ] **AC-02-22** If the player has set availability preferences, each event shows a badge: "Matches your availability" or "Conflicts with your availability" — § "US-02.06..." (lines 310-312)

**US-02.07 — Player RSVPs to Event** (line 318)
- [ ] **AC-02-23** Player views event details and clicks "RSVP" (free events) or "Register & Pay" (paid events); a child account's RSVP requires parent approval (per Epic-01, US-01.04) regardless of price — § "US-02.07: Player RSVPs to Event" § "Acceptance Criteria (Free Event)" (lines 325-327) and § "Acceptance Criteria (Paid Event - Money or Tokens)" (lines 338-340; also epic-level AC § "RSVP & Payment", line 934)
- [ ] **AC-02-24** For a free event with space available, RSVP is instantly confirmed: status "Registered," the event is added to "My Reservations," a "You're registered!" confirmation message is shown, and a confirmation email is sent — § "US-02.07..." § "Acceptance Criteria (Free Event)" (lines 328-332; also epic-level AC § "RSVP & Payment", line 930)
- [ ] **AC-02-25** For a paid event (money or tokens), the player is shown the price ($X or Y tokens), redirected to payment (Epic-05) to enter a payment method or use tokens, and the payment is processed — § "US-02.07..." § "Acceptance Criteria (Paid Event - Money or Tokens)" (lines 338, 341-343)
- [ ] **AC-02-26** After a paid event's payment succeeds, the RSVP is confirmed with status "Registered," the event is added to "My Reservations," a confirmation message and email are sent, and a receipt is provided — § "US-02.07..." § "...Paid Event..." (lines 344-348; also epic-level AC § "RSVP & Payment", line 931)
- [ ] **AC-02-27** If the event is full, the player sees "Event Full - No spots available" (free) or "Event Full" (paid), the RSVP/registration control is disabled, and the player is suggested to check back later or contact the trainer — § "US-02.07..." (lines 333-335, 349; also epic-level AC § "RSVP & Payment", line 932)
- [ ] **AC-02-28** RSVP validation: a player cannot RSVP twice to the same event (shown "Already registered"), cannot RSVP if ineligible by age/skill/gender, and cannot RSVP to past events — § "US-02.07..." § "Validation" (lines 352-354)

**US-02.08 — Player Cancels RSVP** (line 358)
- [ ] **AC-02-29** Player cancels an RSVP for a free event from "My Reservations" via "Cancel RSVP," confirms ("Are you sure? This will free your spot."), and on confirmation the RSVP is canceled, the player is removed from the event roster, the spot opens for other players, and a cancellation email is sent — § "US-02.08: Player Cancels RSVP" § "Acceptance Criteria (Free Event)" (lines 365-371; also epic-level AC § "RSVP & Payment", line 933)
- [ ] **AC-02-30** A player cannot cancel an RSVP after the event's start time — § "US-02.08..." § "...(Free Event)" (line 372). (The epic also floats an earlier, unresolved cutoff, "or X hours before — policy TBD"; see Open questions.)
- [ ] **AC-02-31** Canceling a paid event's RSVP follows the same flow as a free event, plus refund processing under the 24-hour refund policy: full refund (money or tokens) if canceled ≥24 hours before the event, no refund if <24 hours before, and the trainer can manually override via Stripe if needed — § "US-02.08..." § "Acceptance Criteria (Paid Event)" (lines 375-380)
- [ ] **AC-02-32** A processed refund is handled automatically (Epic-05) and a refund confirmation email is sent — § "US-02.08..." § "...(Paid Event)" (lines 381-382; also epic-level AC § "Data Integrity", line 952)
- [ ] **AC-02-33** If a child account initiates the cancellation, parent approval is required, the same as for a purchase — § "US-02.08..." § "Special Case" (line 384)

**US-02.10 — Coach Confirms Event Assignment** (line 388) — note: US-02.09 does not exist in the source; see Open questions.
- [ ] **AC-02-34** Coach navigates to "My Activities," where an "Events to Confirm" section lists pending assignments, each showing title, date/time, location, and number of players RSVP'd — § "US-02.10: Coach Confirms Event Assignment" (lines 395-397; also epic-level AC § "Coach Experience", line 937)
- [ ] **AC-02-35** Coach confirms an assignment via "Confirm": the session moves to "Assigned Sessions," status becomes "Confirmed," the trainer is notified (in-app + email), and the event shows in the coach's calendar — § "US-02.10..." § "Option A - Confirm" (lines 400-405; also epic-level AC § "Coach Experience", line 938)
- [ ] **AC-02-36** Coach declines an assignment via "Decline" with an optional text reason: the session is removed from the coach's view, the trainer is notified (in-app + email, "[Coach] declined [Event]. Find replacement."), and the event still exists, needing a new coach assignment — § "US-02.10..." § "Option B - Decline" (lines 407-412)
- [ ] **AC-02-37** As an optional trainer preference setting, an assignment the coach has not responded to within 48 hours auto-confirms — § "US-02.10..." (line 414; see Open questions, Q-02.02)

**US-02.11 — Coach Tracks Attendance** (line 418)
- [ ] **AC-02-38** After an event's start time, the coach opens the event from "My Activities" and a "Take Attendance" button becomes available, showing a player roster with attendance options per player: Present (default if the player RSVP'd), Absent, Late, or Excused — § "US-02.11: Coach Tracks Attendance" (lines 425-431; also epic-level AC § "Coach Experience", line 939)
- [ ] **AC-02-39** Coach selects an attendance status for each player and saves, recording attendance — § "US-02.11..." (lines 432-433)
- [ ] **AC-02-40** Recorded attendance is visible to the trainer (in event details and analytics), Super Admin (in reports), and optionally the player/parent (in their history) — § "US-02.11..." (lines 434-437; also "Implementation Notes — Notification Requirements", line 1072, "Attendance recorded (optional: notify players)")
- [ ] **AC-02-41** Attendance edit permissions: the coach can edit attendance the same day only (locked after midnight), while the trainer and Super Admin can edit at any time (trainer edits override the coach's entry) — § "US-02.11..." § "Edit Rules" (lines 440-443; also epic-level AC § "Coach Experience", lines 940-941; § "Data Integrity", line 951)
- [ ] **AC-02-42** Attendance validation: attendance cannot be marked for events that have not started yet or for events not assigned to the coach, and a player who canceled their RSVP is not shown in the attendance list — § "US-02.11..." § "Validation" (lines 446-448)

**US-02.12 — Trainer Views RSVP List** (line 452)
- [ ] **AC-02-43** Trainer opens an event from Event Builder and views the "RSVP List" tab, showing for each registrant: player name, age, skill level, RSVP timestamp, payment status (for paid events: Paid, Pending, Refunded), and an availability match indicator (green/yellow/red, from Epic-01 player availability data) — § "US-02.12: Trainer Views RSVP List" (lines 459-464)
- [ ] **AC-02-44** The RSVP list shows a registration count with capacity ("15 of 20 registered"), and displays "Event Full - No spots available" once full — § "US-02.12..." (lines 465-466)
- [ ] **AC-02-45** From the RSVP list, the trainer can manually add a player (if space available), remove a player (issuing a refund if the event is paid), and export the list as CSV — § "US-02.12..." (lines 467-470)

**US-02.13 — Trainer Cancels Event** (line 474)
- [ ] **AC-02-46** Trainer selects an event in Event Builder and clicks "Cancel Event"; the system asks for confirmation ("This will notify all registered players and process refunds. Continue?") and requires a cancellation reason — § "US-02.13: Trainer Cancels Event" (lines 481-484)
- [ ] **AC-02-47** On confirmed cancellation: the event's status becomes "Canceled," all registered players are notified (email + in-app), refunds are processed automatically for paid events per the 24-hour refund policy (Epic-05, D-BUS-005), the event is removed from (or shown as "Canceled" in) the Training Calendar, and the assigned coach is notified that the assignment is canceled — § "US-02.13..." (lines 485-490; also epic-level AC § "Event Creation (Trainer)", line 920)
- [ ] **AC-02-48** Super Admin can cancel any event at any time, overriding the trainer cancellation-window policy — § "US-02.13..." § "Edge Cases" (line 494)
- [ ] **AC-02-49** An event cannot be canceled after it has already started or completed; it is marked "Completed" instead — § "US-02.13..." § "Edge Cases" (line 495)

**US-02.14 — Trainer Edits Event** (line 499)
- [ ] **AC-02-50** Trainer selects an event in Event Builder and clicks "Edit"; all fields are editable for events that are not in the past — title, description, date/time, location, capacity, coach, pricing, eligibility — § "US-02.14: Trainer Edits Event" (lines 506-508)
- [ ] **AC-02-51** Editing triggers targeted notifications: a date/time or location change notifies all RSVP'd players; a coach change notifies both the old and new coach; a price change notifies RSVP'd players, and if the price increased, allows them to cancel with a refund — § "US-02.14..." (lines 509-513; also epic-level AC § "Event Creation (Trainer)", line 919)
- [ ] **AC-02-52** Saving applies the changes, and the updated event details are reflected everywhere the event appears (calendar, My Reservations, My Activities) — § "US-02.14..." (lines 514-515)
- [ ] **AC-02-53** Increasing capacity opens more spots for new RSVPs; decreasing capacity below the current RSVP count shows a warning ("This will exceed capacity. Remove players or increase capacity.") and blocks saving until resolved — § "US-02.14..." § "Capacity Changes" (lines 518-521)
- [ ] **AC-02-54** Edit validation: events in the past cannot be edited, canceled events cannot be edited (a new event must be created instead), and date/time changes are validated (end after start, not in the past) — § "US-02.14..." § "Validation" (lines 524-526)

**US-02.15 — Super Admin Views All Events** (line 530)
- [ ] **AC-02-55** Super Admin navigates to "Event Master Tool" and sees a list of all events from all trainers, with tool-specific search by title, trainer name, location, or date — § "US-02.15: Super Admin Views All Events" (lines 537-539, 555; also epic-level AC § "Super Admin", line 944)
- [ ] **AC-02-56** Super Admin can filter the event list by trainer, location, date range, event type (Training, Private, Small Group), and status (Active, Canceled, Completed), with pagination showing 50 events per page — § "US-02.15..." (lines 540-546; also epic-level AC § "Super Admin", line 945)
- [ ] **AC-02-57** Clicking an event opens its full details; Super Admin can edit any event (same as a trainer), cancel any event (overriding trainer restrictions), override scheduling conflicts without warnings, view the RSVP list, and export events as CSV — § "US-02.15..." (lines 547-553, 555; also epic-level AC § "Super Admin", lines 946-947)

**Scope notes (not dedicated user stories)**
- [ ] **AC-02-58** An event can be configured with both a USD price and a token price at the same time ("Dual Pricing"); the trainer can toggle either pricing option on or off per event, and the player then chooses which to pay with at RSVP; by default, token pricing is enabled (1 token) and USD pricing is disabled ($0) — § "In Scope (MVP) — Event Creation & Management (Trainer)" § "Dual Pricing Options" (lines 44-48). Cross-epic dependency: the payment/token processing itself belongs to Epic-05 (Epic-02's own "Dependencies", line 143, and US-02.07, line 341, both route payment handling to Epic-05); this criterion covers only that the event carries two prices and the player picks one.
- [ ] **AC-02-59** Events can be priced at more than one token — token pricing is not fixed at 1:1 — and the token amount is trainer-configurable per event (e.g., a premium 3-hour session could cost 2 tokens) — § "...Event Creation & Management (Trainer)" § "Flexible Token Pricing" (lines 49-52)
- [ ] **AC-02-60** When an event has both USD and token pricing enabled, the player is shown both options clearly (e.g., "Pay $25 OR 2 tokens") and chooses a payment method at RSVP time — § "In Scope (MVP) — Player/Parent Experience" § "Dual Payment Options" (lines 74-77; also § "US-02.07: Player RSVPs to Event" § "Acceptance Criteria (Paid Event - Money or Tokens)", line 341)
- [ ] **AC-02-61** Trainer defines a recurring pattern (e.g., "Every Tuesday for 3 months"), and the platform bulk-generates individual event instances from it (e.g., 12 separate Tuesday events); each generated event is independent — it can have a different coach and can be edited or canceled separately from the others — § "...Event Creation & Management (Trainer)" § "Recurring Events - Bulk Auto-Creation" (lines 53-56). This is a distinct feature from Duplicate Event (US-02.05, AC-02-14..17) — see Open questions (analyst-raised) for the resulting scope conflict with `Epic_Areas_Plan.md`.
- [ ] **AC-02-62** Players holding a qualifying subscription automatically get access to each event generated by a recurring bulk-creation pattern — § "...Recurring Events - Bulk Auto-Creation" (line 57). Cross-epic dependency: the subscription itself is Epic-05's; this criterion covers only automatic RSVP-eligibility for each generated event.
- [ ] **AC-02-63** Players with an unlimited subscription can RSVP to only 1 event per day in advance; same-day bookings are unlimited, subject to spots being available; attempting to exceed the advance limit shows "Subscription holders can book 1 event per day in advance" — § "In Scope (MVP) — Player/Parent Experience" § "Subscription RSVP Restrictions" (lines 78-82). Cross-epic dependency: the subscription and its terms are Epic-05's; this criterion covers only the RSVP-frequency limit Epic-02 enforces.
- [ ] **AC-02-64** Players who train with multiple trainers see fully separated Training Calendar views per trainer context, switching between them rather than seeing one combined calendar — § "In Scope (MVP) — Player/Parent Experience" (line 71; also epic-level AC § "Event Discovery (Player/Parent)", line 926). Cross-epic dependency: the context-switching mechanism itself is Epic-01's (US-01.02); this criterion covers its effect on the Training Calendar specifically.
- [ ] **AC-02-65** While a coach assignment awaits confirmation, the event displays "Pending Coach Confirmation" until the coach accepts — § "...Event Creation & Management (Trainer)" § "Coach assignment with per-event approval workflow" (line 66; also "Business Rules & Logic — Coach Assignment Rules" § "Assignment Process", line 703, "Status: 'Pending' until coach confirms")

**Epic-level (from "Acceptance Criteria (Epic-Level)", line 909) — not already covered above**
- [ ] **AC-02-66** The Training Calendar view works on both desktop and mobile — § "Acceptance Criteria (Epic-Level)" § "Event Discovery (Player/Parent)" (line 927)
- [ ] **AC-02-67** RSVP count never exceeds an event's capacity — capacity management holds even under concurrent registration attempts for the same spot — § "Acceptance Criteria (Epic-Level)" § "Data Integrity" (line 950; also "Implementation Notes — Security Requirements", line 1062, "Prevent RSVP capacity bypass attacks (race conditions)")
- [ ] **AC-02-68** Canceled events are preserved as history rather than deleted — § "Acceptance Criteria (Epic-Level)" § "Data Integrity" (line 953; also "Business Rules & Logic — Cancellation & Refund Rules" § "Trainer Cancels Event", line 693)
- [ ] **AC-02-69** Performance: the Training Calendar loads in <3 seconds with 100 events; event creation completes in <2 seconds; RSVP confirmation completes in <1 second for free events and <3 seconds for paid events (including payment); the platform handles 50 concurrent RSVPs to the same event — § "Acceptance Criteria (Epic-Level)" § "Performance" (lines 956-959; also "Performance & Scale Targets", lines 883-893 — see Open questions on additional narrative-only targets)
- [ ] **AC-02-70** (Process gate, not product behavior) Epic-02 is considered complete only once the demo is approved, all P0 and P1 open questions are resolved, the refund policy is confirmed, and private event functionality is validated — § "Acceptance Criteria (Epic-Level)" § "Approval" (lines 962-965)

**Final count: 70 acceptance criteria (AC-02-1 .. AC-02-70)**, covering all
14 existing user stories (US-02.01–US-02.08 and US-02.10–US-02.15 —
US-02.09 does not exist in the source; see Open questions), 8 additional
criteria derived from "In Scope (MVP)" bullets that have no dedicated
story, and the epic-level "Acceptance Criteria (Epic-Level)" completion
checklist, with 27 epic-level items folded into their matching story or
scope criterion rather than duplicated, and the remaining 11 forming
AC-02-66 through AC-02-70.

## Business rules

Restated from "Business Rules & Logic" (line 618). The epic groups these
under eight named categories — Event Creation, Visibility, RSVP,
Cancellation & Refund, Coach Assignment, Attendance, Duplication, and
Player Availability Integration — all eight are represented below; none
are dropped.

- [ ] **BR-02-1** Required fields to create an event: title (max 100 characters), event type, date and time (start and end), location, and capacity (minimum 1) — `Epic-02_Event_Management_Scheduling_SPEC.md` § "...Event Creation Rules" § "Required Fields" (lines 623-627)
- [ ] **BR-02-2** Date/time validation: start time cannot be in the past, end time must be after start time, and duration should be reasonable (a warning above 8 hours, a hard maximum of 24 hours) — § "...Event Creation Rules" § "Date/Time Validation" (lines 630-632)
- [ ] **BR-02-3** Pricing rules: a paid event's amount must be > 0, a free event's amount is 0, and a token-priced event's token amount must be > 0 — § "...Event Creation Rules" § "Pricing Rules" (lines 635-637)
- [ ] **BR-02-4** Capacity rules: capacity must be a positive integer between 1 and 999, and capacity cannot be reduced below the current RSVP count — § "...Event Creation Rules" § "Capacity Rules" (lines 640-642)
- [ ] **BR-02-5** Public events are shown to all eligible players on the Training Calendar, with eligibility determined by age, skill level, and gender restrictions; an event with no restrictions is shown to all players associated with the trainer — § "...Event Visibility Rules" § "Public Events" (lines 647-649)
- [ ] **BR-02-6** Private events are shown only to invited players, even if a non-invited player would otherwise be eligible by age/skill/gender; private events cannot be discovered via search by non-invited players, and direct-link access for non-invited players shows "Access Denied" — § "...Event Visibility Rules" § "Private Events" (lines 652-655)
- [ ] **BR-02-7** General RSVP rules: a player can RSVP only once per event, cannot RSVP to past events, cannot RSVP once capacity is reached, and must meet eligibility criteria (age, skill, gender) — § "...RSVP Rules" § "General" (lines 660-663)
- [ ] **BR-02-8** Free events RSVP instantly with no payment step and immediate confirmation — § "...RSVP Rules" § "Free Events" (lines 666-667)
- [ ] **BR-02-9** Paid events require payment before confirmation; the RSVP is not confirmed until payment succeeds, and if payment fails the RSVP is not registered and the spot remains available — § "...RSVP Rules" § "Paid Events" (lines 670-672)
- [ ] **BR-02-10** Child accounts require parent approval for all RSVPs (free or paid); the request status is "Pending Parent Approval" until the parent confirms, with a 48-hour expiry on the approval request — § "...RSVP Rules" § "Child Accounts" (lines 675-677)
- [ ] **BR-02-11** A player cannot cancel an RSVP after the event has started; cancellation ≥24 hours before the event yields a full refund (money or tokens), cancellation <24 hours before yields no refund, and the trainer can manually override via Stripe to issue a refund if needed — § "...Cancellation & Refund Rules" § "24-Hour Refund Policy — Player Cancels RSVP" (lines 684-687). Cross-epic dependency: the refund policy's execution (issuing money/token refunds) is Epic-05's; this rule states only when Epic-02 treats a cancellation as refund-eligible.
- [ ] **BR-02-12** When a trainer cancels an event, all players always receive a full refund regardless of timing, all players are notified, coach assignments are canceled, and the event is marked "Canceled" rather than deleted, preserving history — § "...Cancellation & Refund Rules" § "24-Hour Refund Policy — Trainer Cancels Event" (lines 690-693). Cross-epic dependency: refund execution is Epic-05's; this rule states only the trainer-cancellation refund entitlement.
- [ ] **BR-02-13** A trainer assigns a coach to an event; the trainer may assign themselves or any coach added via Epic-01; if the assignment conflicts with the coach's stated availability, a warning is shown and the trainer can override with a reason; the coach is notified and the assignment status is "Pending" until the coach confirms — § "...Coach Assignment Rules" § "Assignment Process" (lines 698-703)
- [ ] **BR-02-14** The coach can confirm or decline the assignment; a decline notifies the trainer to find a replacement; as an optional trainer setting, the assignment auto-confirms if the coach does not respond within 48 hours — § "...Coach Assignment Rules" § "Coach Confirmation" (lines 706-708)
- [ ] **BR-02-15** A coach cannot be assigned to overlapping events; the system warns on double-booking, and the trainer can override with a logged reason — § "...Coach Assignment Rules" § "Coach Limitations" (lines 711-713)
- [ ] **BR-02-16** Attendance can be marked only after an event's start time, only by the coach assigned to that event, and defaults to "Present" for all RSVP'd players — § "...Attendance Tracking Rules" § "Recording Attendance" (lines 718-720)
- [ ] **BR-02-17** Attendance statuses are Present (attended), Absent (no-show), Late (arrived late but participated), and Excused (excused absence, coach discretion) — § "...Attendance Tracking Rules" § "Attendance Statuses" (lines 723-726)
- [ ] **BR-02-18** Attendance edit permissions: the coach can edit only the same day (until midnight, after which the coach's view becomes read-only); the trainer and Super Admin can edit at any time — § "...Attendance Tracking Rules" § "Edit Permissions" (lines 729-732)
- [ ] **BR-02-19** Duplicating an event copies all event details (title, type, location, coach, capacity, pricing, eligibility) but not RSVPs or attendance records; the new event receives a new unique identifier, its date/time must be changed, and a reference to the original event is stored for analytics — § "...Event Duplication Rules" § "Duplication Process" (lines 737-741)
- [ ] **BR-02-20** A player's stated availability (from Epic-01) is compared to the event's date/time and shown to the trainer only (not to players) as a match indicator: green if the event is fully within the player's available times, gray if the player hasn't set availability for that time, or red if the event conflicts with times the player marked unavailable — § "...Player Availability Integration Rules" § "Availability Matching" (lines 746-751). Cross-epic dependency: the underlying availability data is Epic-01's (US-01.09); this rule covers only how Epic-02 consumes and displays it.

## Data requirements

Restated from "Data Requirements" (line 559) — entities, the fields the
epic names, and the relationships it names. No schema, keys, or types are
proposed here.

- **Event**: unique identifier, title, description, event type (Training Session, Private Session, Small Group), the trainer who created it, start and end date/time, location, assigned coach(es), capacity (maximum RSVPs), current RSVP count, visibility (Public, Private/Invite-Only), the list of invited players/groups if private, pricing type (Free, Paid Money, Paid Tokens), amount if paid, eligibility restrictions (age range, skill level, gender), status (Active, Canceled, Completed), created/updated timestamps, and — if canceled — the reason, who canceled it, and when. Relationship: created by a Trainer; optionally has assigned Coach(es); if private, restricted to specific invited Players. (line 563)
- **RSVP**: which player, which event, RSVP timestamp, status (Registered, Canceled, Attended, No-Show), and — if paid — payment reference and refund status, and — if canceled — cancellation timestamp and reason. Relationship: references a Player and an Event. (line 582)
- **Attendance**: which event, which player, attendance status (Present, Absent, Late, Excused), who recorded it (coach), recorded timestamp, and — if edited — edit history (who, when, old value, new value). Relationship: references an Event, a Player, and the recording Coach. (line 590)
- **Coach Assignment**: which event, which coach, assignment status (Pending, Confirmed, Declined), reason if declined, confirmation timestamp, and — if there was an availability conflict — an override flag, override reason, and who overrode it. Relationship: references an Event and a Coach. (line 598)
- **Private Event Invitation**: which event, which player or group invited, invitation timestamp. Relationship: references an Event and an invited Player (or group). (line 606)
- **Event Duplication record**: original event reference (for tracking/analytics), duplication timestamp, who duplicated it. Relationship: references the original Event and the newly created Event. (line 611)

## Edge cases

| Case | Expected |
|---|---|
| Event reaches capacity while players are trying to register | "Event Full - No spots available" shown, RSVP disabled, suggested to check back later or contact the trainer directly; if a registered player later cancels, the spot reopens, available first-come-first-served — US-02.07 (lines 333-335); Flow 3 "Event Full - Player Cannot Register" (lines 797-807) |
| Player attempts to RSVP twice to the same event | Blocked, shown "Already registered" — US-02.07 "Validation" (line 352) |
| Player attempts to RSVP while ineligible (age/skill/gender) or to a past event | Blocked — US-02.07 "Validation" (lines 353-354); Business Rules "RSVP Rules — General" (lines 661, 663) |
| Paid-event payment fails during RSVP | RSVP not registered; spot remains available — Business Rules "RSVP Rules — Paid Events" (line 672) |
| Player cancels a paid event's RSVP | ≥24 hours before the event: full refund (money or tokens); <24 hours before: no refund; trainer can manually override via Stripe — US-02.08 "Acceptance Criteria (Paid Event)" (lines 376-380); Business Rules "...Player Cancels RSVP" (lines 684-687) |
| Trainer cancels an event | Always a full refund to all players regardless of timing; all players notified; coach assignments canceled; event marked "Canceled," not deleted — US-02.13 (lines 485-490); Business Rules "...Trainer Cancels Event" (lines 690-693) |
| Trainer assigns a coach to a time conflicting with the coach's stated "My Times" availability | Warning shown; trainer must supply a reason to override; override logged (who, when, event, coach, reason) — US-02.03 (lines 231-234); Business Rules "Coach Assignment Rules — Assignment Process" (line 701) |
| Coach declines an event assignment | Trainer notified to find a replacement; event still exists and needs a new coach assignment — US-02.10 "Option B - Decline" (lines 408-412) |
| Coach attempts to mark attendance before the event has started, or for an event not assigned to them | Blocked — US-02.11 "Validation" (lines 446-447) |
| Coach attempts to edit attendance after midnight of the event day | Locked to the coach (read-only); trainer and Super Admin can still edit — US-02.11 "Edit Rules" (lines 440-442); Business Rules "Attendance Tracking Rules — Edit Permissions" (lines 729-732) |
| A player who canceled their RSVP is checked for attendance | Not shown in the attendance list — US-02.11 "Validation" (line 448) |
| Non-invited player accesses a private event via a direct link | "Access Denied" — US-02.02 (line 211); Business Rules "Event Visibility Rules — Private Events" (line 655) |
| Trainer decreases event capacity below the current RSVP count | Warning shown ("This will exceed capacity. Remove players or increase capacity."); cannot save until resolved — US-02.14 "Capacity Changes" (lines 519-521); Business Rules "Event Creation Rules — Capacity Rules" (line 642) |
| Trainer attempts to edit a past or canceled event | Blocked; a canceled event requires creating a new event instead — US-02.14 "Validation" (lines 524-525) |
| Trainer attempts to cancel an event that already started or completed | Blocked; marked "Completed" instead — US-02.13 "Edge Cases" (line 495) |
| Super Admin cancels or edits any event, or overrides a scheduling conflict | Allowed at any time, overriding trainer restrictions and conflict warnings — US-02.13 "Edge Cases" (line 494); US-02.15 "Actions" (lines 549-551) |
| Child account initiates an RSVP or a cancellation | Requires parent approval — US-02.07 (lines 327, 340); US-02.08 "Special Case" (line 384) |
| Trainer raises the price on an event with existing RSVPs | Affected players are notified; if the price increased, they may cancel with a refund — US-02.14 (line 513) |

Boundary/failure behaviors the epic raises but does **not** state a settled
expected outcome for — the exact RSVP-cancellation cutoff beyond "not after
the event starts" (line 372, "policy TBD") and the exact event-cancellation
cutoff beyond "not after it starts/completes" (line 493, "policy TBD") —
are listed in Open questions instead of guessed here.

## Out of MVP scope

Verbatim from "Out of Scope (Post-MVP)" (line 105):

**Confirmed OUT of MVP**:
- ❌ Open Gym / League events
- ❌ League games and schedules
- ❌ Referee assignments

**Moved to Epic-08**:
- ✅ Camps & Evaluations - Now in Epic-08 (Forms & Registration)
  - Camps are FORMS, not calendar events
  - See Epic-08 for camp registration system

**Phase 2 Deferrals**:
- ❌ Event Waitlist - Simplified for MVP: Show "Event Full" only, no waitlist queue
- ❌ Per-trainer refund policy configuration - Using global 24-hour refund policy instead
- ❌ True recurring event series - Using bulk auto-creation instead (creates individual events, not linked series)
- ❌ Bulk operations (bulk edit, bulk cancel, bulk clone)
- ❌ Event templates (saved event configurations)
- ❌ Advanced scheduling suggestions (AI-powered)
- ❌ Automatic reminders (email/SMS 24hr before)
- ❌ Player feedback/ratings after events
- ❌ Video conferencing integration (virtual events)
- ❌ Equipment tracking (equipment required/provided)

Two items in this verbatim list interact with conflicts recorded in Open
questions (analyst-raised), reproduced here exactly as written: "True
recurring event series" excludes only linked/auto-generating series, not
the "Recurring Events - Bulk Auto-Creation" feature that Section 3 lists as
in-scope (lines 53-57, captured as AC-02-61/62) — see Open questions.
"Bulk operations" is excluded here but reappears as in-scope in
`Epic_Areas_Plan.md` — see Open questions.

## Open questions

### From the epic's "Questions / Open Issues" section (line 899, verbatim)

| ID | Question | Priority | Status | Owner |
|:---|:---|:---:|:---|:---|
| Q-02.02 | Auto-confirm coach assignments after 48 hours, or require explicit confirmation always? | P2 | Open | Client |
| Q-02.08 | Event reminders: Email/SMS 24hrs before? (Defer to Post-MVP?) | P2 | Open | Client |
| Q-02.09 | Can trainer manually add player to full event (override capacity)? | P2 | Open | Client |

### Analyst-raised

- **(analyst-raised)** US-02.09 does not exist. The epic's user stories run
  US-02.01–US-02.08, then jump to US-02.10–US-02.15 — 14 stories total. A
  direct search of `Epic-02_Event_Management_Scheduling_SPEC.md` confirms
  no US-02.09 appears anywhere. `Epic_Areas_Plan.md`'s "Epic Summary" table
  nonetheless credits Epic-02 with 15 stories ("Epic-02: Event Management |
  15 |", line 556) against Epic-01's correctly-counted 14 (line 555). This
  spec does not renumber the later stories to hide the gap and does not
  invent a replacement US-02.09 — the story is simply missing from the
  source, and the plan's count is wrong.
- **(analyst-raised)** Recurring events conflict between the epic and the
  plan. `Epic_Areas_Plan.md`'s Epic-02 section states "❌ NO recurring
  event series (deferred to Phase 2)" as MVP Scope (line 120) and offers
  only "Duplicate Event feature (replaces recurring events for MVP)" (line
  116; also Key Features, line 100). But the epic file itself lists
  "Recurring Events - Bulk Auto-Creation" as In Scope (MVP) under "Event
  Creation & Management (Trainer)" (lines 53-57): the trainer defines a
  recurring pattern and the platform bulk-generates independent event
  instances, each separately editable/cancelable, with automatic
  subscription-holder access. The epic's own Out-of-Scope list (line 120)
  excludes only "True recurring event series ... using bulk auto-creation
  instead (creates individual events, not linked series)" — a narrower
  exclusion than the plan's blanket framing implies. This spec follows the
  epic and specifies bulk auto-creation as in-scope (AC-02-61, AC-02-62);
  the plan's "NO recurring events" framing conflicts with it.
- **(analyst-raised)** Event types conflict between the epic and the plan.
  The epic states three event types — "Training Session, Private Session,
  Small Group" (line 42) — while `Epic_Areas_Plan.md` states three
  different ones — "Training Sessions, Camps, Privates" (Epic-02 section,
  "MVP Scope", line 110; also "Overview", line 91). The epic's own
  "Out of Scope (Post-MVP)" section resolves this: Camps are "Now in
  Epic-08 (Forms & Registration)... Camps are FORMS, not calendar events"
  (lines 112-115). This spec follows the epic's three event types
  throughout (e.g., AC-02-1); the plan's "Camps" item is stale relative to
  the epic.
- **(analyst-raised)** Bulk operations conflict between the epic and the
  plan. The epic explicitly excludes bulk operations from MVP — "Bulk
  operations (bulk edit, bulk cancel, bulk clone)" (line 121) — while
  `Epic_Areas_Plan.md` lists "Bulk operations (simplified)" as in-scope
  (Epic-02 section, "MVP Scope", line 119) and "Bulk operations (Super
  Admin: clone, edit, cancel)" as a key feature (line 102). This spec
  follows the epic and specifies only one-at-a-time Super Admin actions
  (AC-02-57). Note that Epic-07's own scope summary still describes the
  Event Master Tool as "system-wide event management"
  (`Epic-07_Super_Admin_System_Management_SPEC.md` § "Scope Summary", line
  29) even though Epic-07 elsewhere commits to "Bulk operations (one-by-one
  for MVP)" (same file, line 36) and defers bulk trainer operations
  post-MVP ("Q-07.03: NO - Bulk operations deferred to Post-MVP", line
  589) — the "system-wide" phrasing is a residual ambiguity worth
  confirming does not imply bulk event actions.
- **(analyst-raised)** US-02.08's free-event cancellation criteria state a
  player "Cannot cancel after event start time (or X hours before - policy
  TBD)" (line 372) — the epic flags its own cutoff as undecided. No AC is
  derived for the parenthetical "X hours before" alternative; only the
  certain "not after event start" cutoff is specified (AC-02-30).
- **(analyst-raised)** US-02.13's trainer-cancellation edge cases state a
  trainer "Cannot cancel event less than X hours before start (policy TBD,
  e.g., 24 hours)" (line 493) — again flagged undecided by the epic itself
  (the "24 hours" is only an example, not a stated rule). No AC is derived
  for this cutoff; it does not appear in this spec's Acceptance criteria or
  Edge cases.
- **(analyst-raised)** "Event notes (coach preparation notes)" is listed as
  in-scope under Coach Experience (line 95), but no user story, acceptance
  criterion, business rule, or data field anywhere else in the epic
  describes who creates or views these notes, when, or how — the epic's
  own Data Requirements for Events (line 563) do not list a notes field.
  No AC is derived for this scope item; it is reported here as a gap
  rather than invented.
- **(analyst-raised)** "Event analytics and reporting" is listed in-scope
  for Super Admin Tools (line 101), but the only concrete elaboration found
  anywhere in the epic is CSV export of the event list (line 553, captured
  in AC-02-57). No acceptance criterion, business rule, or data requirement
  states what analytics content Super Admin should see. No further AC is
  derived beyond CSV export; the rest of this scope item is reported as a
  gap.
- **(analyst-raised)** The epic's own "Questions / Open Issues" table
  contains only Q-02.02, Q-02.08, and Q-02.09 — it skips Q-02.01 and
  Q-02.03 through Q-02.07 (six IDs), and none of them appear anywhere else
  in the epic. Unclear whether questions were removed, renumbered, or
  never filled in — the same kind of numbering gap the Epic-01 spec
  flagged for its own open-questions table (one skipped ID there; six
  skipped here).
- **(analyst-raised)** Minor inconsistency between the two places
  performance targets are stated: the narrative "Performance & Scale
  Targets" section (lines 883-893) includes two items not repeated in the
  epic-level "Acceptance Criteria (Epic-Level) — Performance" checklist
  (lines 956-959) — "Event search: <1 second for query results" (line 887)
  and the two Scale bullets "Support 10,000+ events per trainer
  (historical)" (line 890) and "Support 1,000+ players RSVPing across
  multiple events simultaneously" (line 893). All four checklist items are
  also present in the narrative section, so there is no contradiction —
  just incomplete overlap, worth reconciling into one list (the same
  pattern the Epic-01 spec flagged for its own performance sections).
