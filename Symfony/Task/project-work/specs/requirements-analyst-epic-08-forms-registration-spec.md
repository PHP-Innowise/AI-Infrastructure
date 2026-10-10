# Spec: Epic-08 — Forms & Registration (Camps/Evaluations)

Source: `Task/Epics/Epic-08_Forms_Registration_SPEC.md` (read-only client material)

> Citation note: Epic-08 has no single "Acceptance Criteria" section; its
> acceptance criteria are drawn from the per-story **Acceptance Criteria**
> lists inside §7 "User Stories" (line 171) and from §13 "Success Criteria"
> (line 461). As in the Epic-02/05/06 specs, several rule/criteria groups
> also sit under **bold sub-labels** rather than markdown headings; those
> are cited as a second `§` level — `§ "Parent Heading" § "Sub-label"`. Two
> of those sub-labels collide with unrelated numbered headings elsewhere in
> the file (see Open questions, analyst-raised, methodology note). Every
> back-reference below therefore cites heading **text** plus a line number,
> never a bare section number.

## Problem

Epic-08 gives trainers a forms-based registration system for camps and
evaluations that the epic explicitly separates from the calendar-based
events built in Epic-02: "Forms" are defined as "Data collection tools with
shareable links" and "Calendar Events (Epic-02)" as "Time-scheduled
activities with RSVP" ("Description — Key Distinction", lines 9-12).
Epic-02's own scope list confirms the split at the source: camps and
evaluations were moved out of Epic-02 into this epic because "Camps are
FORMS, not calendar events" (`Epic-02_Event_Management_Scheduling_SPEC.md`
§ "Out of Scope (Post-MVP) — Moved to Epic-08", lines 112-115). Before this
epic, the platform can market and register only people who already have
accounts; trainers need to promote camps on social media, collect
registrations from people who are not platform users yet, process payments
for camps, and convert camp participants into regular customers ("Business
Value — Problem Statement", lines 30-35). This epic addresses that with a
template-based form builder for two form types — temporary, capacity-limited,
toggleable Camps, and permanent, uncapped Evaluations — each reachable by a
public shareable link requiring no login, optionally gated by a single
Stripe Checkout price, and ending in an optional, non-forced conversion of
the submitter into a full trainer-linked user account ("Description — Core
Features", lines 13-18). Success is measured by adoption (70%+ of trainers
create at least one form), volume (2-3 camps per trainer per quarter), and
funnel effectiveness (30%+ camp-to-account conversion, camp registrations at
20%+ of new player acquisitions, zero payment processing errors) ("Business
Value — Success Metrics", lines 38-42). Per its own dependency list, this
epic requires Epic-01 to already exist for the account-creation half of the
conversion flow, and Epic-05 to already exist for Stripe Checkout
("Dependencies — Required Before This Epic", lines 132-133); in return it
enables trainer lead generation, camp revenue, and a player-acquisition
funnel for the rest of the platform ("Dependencies — This Epic Enables",
lines 136-138). See Open questions for a conflict between this stated
dependency order and the platform-level dependency diagram in
`Epic_Areas_Plan.md`.

## User scenarios

1. **Trainer** creates a camp registration form so that they can market the
   camp externally and collect registrations.
   Path: Trainer opens "Camps & Evaluations," selects "Create Camp" to start
   from the pre-loaded template, customizes the fields (add/remove/reorder),
   sets a camp name, display-only dates, description, capacity limit, and an
   optional price, previews the form, and publishes it to receive a
   shareable link; the trainer can turn the camp's registration on or off at
   any time.
   Source: US-08.01 "Trainer Creates Camp Form" (line 173)

2. **Trainer** creates an always-available evaluation form so that
   prospective players can request an evaluation at any time.
   Path: Trainer opens "Camps & Evaluations," selects "Create Evaluation" to
   start from the pre-loaded template, customizes the fields, sets an
   evaluation name, description, and optional price, previews it, and
   publishes it to receive a permanent link that stays active indefinitely
   with no on/off toggle.
   Source: US-08.02 "Trainer Creates Evaluation Form" (line 197)

3. **Non-user (parent/player)** registers for a camp via a shared link so
   that they can secure their spot and pay for the camp.
   Path: Submitter clicks the shareable camp link and, without logging in,
   views the camp's details (name, dates, description, price, spots
   remaining), fills the form fields (e.g., name, email, age, emergency
   contact), reviews any included waiver/terms, and submits; a free camp
   confirms registration immediately, a paid camp first redirects the
   submitter to Stripe Checkout, and the submitter then receives a
   confirmation email and is offered the option to create a user account.
   Source: US-08.03 "Non-User Submits Camp Form" (line 219)

4. **Trainer** views all of a camp's registrations so that they can track
   attendance and manage participants.
   Path: Trainer opens the camp's details, views the participant list (name,
   email, age, payment status, submission date, converted-to-user status),
   filters it by payment status or conversion status, exports it to CSV,
   marks attendance per participant, and sends a bulk email message to all
   participants.
   Source: US-08.04 "Trainer Views Camp Submissions" (line 243)

5. **Camp participant** creates a full user account so that they can access
   the training calendar and other platform features.
   Path: After registering for a camp, the participant sees a "Create Your
   Account" prompt, opens a registration form pre-filled with their
   submission data (name, email), sets a password, accepts terms, and the
   account is created and auto-assigned to the trainer, landing on the
   player dashboard; a participant who skips account creation instead
   receives an email with a ShareLink to register later, and a participant
   whose submission email already has an account is prompted to log in
   instead of creating a duplicate.
   Source: US-08.05 "Player Converts Camp Submission to Account" (line 265)

6. **Trainer** edits and manages their forms so that they can update
   information or disable a camp.
   Path: Trainer views the list of all camps/evaluations, edits a form's
   fields, description, or pricing (a form with existing submissions warns
   how many participants are already registered), enables/disables a camp's
   registration, deletes a form after a confirmation prompt (blocked if the
   form has paid registrations until they are refunded), views the
   submission count and remaining spots, and copies the shareable link.
   Source: US-08.06 "Trainer Manages Form Settings" (line 290)

## Acceptance criteria

Every checkbox below is renumbered sequentially as AC-08-1 .. AC-08-46. The
first 32 are drawn from each user story's "Acceptance Criteria" list in §7
"User Stories" (line 171), folding in that same story's "Business Rules"
sub-list wherever it adds a concrete, testable detail (most often a numeric
range) to one of those criteria. The remaining 14 are drawn from the
epic-level "Success Criteria" section, §13 (line 461) — Functional,
Non-Functional, and Business Requirements — restated with their numeric
targets kept exact. This epic's separate, dedicated §9 "Business Rules"
section is restated on its own below as Business rules; it is not a source
for these acceptance criteria.

**US-08.01 — Trainer Creates Camp Form** (line 173)
- [ ] **AC-08-1** Trainer can access a "Camps & Evaluations" section and start a new camp form from a pre-loaded template by selecting "Create Camp" — `Epic-08_Forms_Registration_SPEC.md` § "US-08.01: Trainer Creates Camp Form" (lines 179-180)
- [ ] **AC-08-2** Trainer can customize the camp form's fields (add/remove/reorder), with a minimum of 1 field required (participant name) — § "US-08.01..." (line 181; also "Business Rules," line 190)
- [ ] **AC-08-3** Trainer sets a camp name, display-only dates not linked to the calendar, and a description — § "US-08.01..." (line 182; also "Business Rules," line 193)
- [ ] **AC-08-4** Trainer sets a capacity limit for the camp (e.g., max 50 participants), constrained to a range of 1-1000 participants — § "US-08.01..." (line 183; also "Business Rules," line 191)
- [ ] **AC-08-5** Trainer can optionally set a price integrated with Stripe: either $0 (free) or in the $1-$10,000 range — § "US-08.01..." (line 184; also "Business Rules," line 192)
- [ ] **AC-08-6** Trainer can preview the form before publishing it — § "US-08.01..." (line 185)
- [ ] **AC-08-7** Publishing the form produces a shareable link — § "US-08.01..." (line 186)
- [ ] **AC-08-8** Trainer can turn a camp's registration on or off (enable/disable) at any time — § "US-08.01..." (line 187)

**US-08.02 — Trainer Creates Evaluation Form** (line 197)
- [ ] **AC-08-9** Trainer can access "Camps & Evaluations" and start a new evaluation form from a pre-loaded template by selecting "Create Evaluation" — § "US-08.02: Trainer Creates Evaluation Form" (lines 203-204)
- [ ] **AC-08-10** Trainer can customize the evaluation's fields (the same field types available to camps), and sets an evaluation name, description, and optional price — § "US-08.02..." (lines 205-207; also "Business Rules," line 215)
- [ ] **AC-08-11** Trainer can preview the form before publishing it — § "US-08.02..." (line 208)
- [ ] **AC-08-12** Publishing produces a permanent link; the evaluation has no capacity limit and remains active indefinitely with no on/off toggle — it can only be deleted, not disabled — § "US-08.02..." (lines 209-210; also "Business Rules," lines 213-214)

**US-08.03 — Non-User Submits Camp Form** (line 219)
- [ ] **AC-08-13** Non-user clicks the shareable camp link, and the form loads showing the camp's details (name, dates, description, price, spots remaining) — § "US-08.03: Non-User Submits Camp Form" (lines 225-226)
- [ ] **AC-08-14** Submitter fills the form fields (e.g., name, email, age, emergency contact) and reviews any waiver/terms included in the form; the email field is validated for format, and the form does not save partial submissions — all fields must be complete before submit — § "US-08.03..." (lines 227-228; also "Business Rules," lines 237, 239)
- [ ] **AC-08-15** If the camp's capacity has been reached, the form shows a "Camp Full" message and does not allow submission; otherwise, submitting confirms registration immediately for a free camp, while a paid camp redirects the submitter to Stripe Checkout first and confirms registration only once payment completes — a pending payment does not count as a registration — § "US-08.03..." (lines 229-231; also "Business Rules," lines 236, 238)
- [ ] **AC-08-16** After a confirmed registration, the submitter receives a confirmation email with the camp's details and is offered the option to create a user account — § "US-08.03..." (lines 232-233)

**US-08.04 — Trainer Views Camp Submissions** (line 243)
- [ ] **AC-08-17** Trainer opens a camp's details and views the participant list, with columns for name, email, age, payment status, submission date, and converted-to-user status (Yes/No) — § "US-08.04: Trainer Views Camp Submissions" (lines 249-251)
- [ ] **AC-08-18** Trainer can filter the participant list by payment status (Paid, Free, Pending) and by conversion status (User Created, Not Converted); by rule, only paid and free registrations are shown — pending payments are excluded — § "US-08.04..." (lines 252-254; also "Business Rules," line 260) — see Open questions for a tension between the "Pending" filter option and the pending-exclusion rule.
- [ ] **AC-08-19** Trainer exports the participant list to CSV — § "US-08.04..." (line 255)
- [ ] **AC-08-20** Trainer marks attendance per participant via checkboxes — § "US-08.04..." (line 256)
- [ ] **AC-08-21** Trainer sends a bulk email message to all of a camp's participants — § "US-08.04..." (line 257)
- [ ] **AC-08-22** Participant data remains visible indefinitely, even after the camp ends — § "US-08.04..." (line 261)

**US-08.05 — Player Converts Camp Submission to Account** (line 265)
- [ ] **AC-08-23** After registering for a camp, the submitter sees a "Create Your Account" prompt; clicking it opens a registration form pre-filled with their submission data (name, email) — § "US-08.05: Player Converts Camp Submission to Account" (lines 271-272)
- [ ] **AC-08-24** Submitter sets a password and accepts terms; the account is created, auto-assigned to the trainer, and the submitter is redirected to the player dashboard, where they can now RSVP for events, purchase content, and use other platform features — § "US-08.05..." (lines 273-277)
- [ ] **AC-08-25** Account creation is optional, not forced; a submitter who skips it instead receives an email containing a ShareLink to register later — § "US-08.05..." § "Alternative Flow" (lines 280-281; also "Business Rules," line 286)
- [ ] **AC-08-26** Before account creation, the system checks whether the submission's email already exists in the system; if it does, the submitter is prompted to log in instead of creating a duplicate account — § "US-08.05..." (lines 284-285)

**US-08.06 — Trainer Manages Form Settings** (line 290)
- [ ] **AC-08-27** Trainer views a list of all their camps and evaluations and can copy each form's shareable link — § "US-08.06: Trainer Manages Form Settings" (lines 296, 301)
- [ ] **AC-08-28** Trainer can edit a form's fields, description, and pricing; editing a form that already has submissions shows a warning naming how many participants are already registered — § "US-08.06..." (line 297; also "Business Rules," line 304)
- [ ] **AC-08-29** Trainer can enable or disable a camp's registration; disabling it makes the shareable link show a "Registration Closed" message instead of the form — § "US-08.06..." (line 298; also "Business Rules," line 307)
- [ ] **AC-08-30** Trainer cannot reduce a camp's capacity limit below its current registration count — § "US-08.06..." § "Business Rules" (line 305)
- [ ] **AC-08-31** Trainer can delete a form after a confirmation prompt; a form with paid registrations cannot be deleted until those payments are refunded via Stripe — § "US-08.06..." (line 299; also "Business Rules," line 306)
- [ ] **AC-08-32** Trainer can view a form's current submission count and remaining spots — § "US-08.06..." (line 300)

**Epic-level (from "Success Criteria," line 461) — not already covered above**
- [ ] **AC-08-33** Trainer can create a camp form in < 10 minutes — § "Success Criteria — Functional Requirements" (line 464)
- [ ] **AC-08-34** A form submission, including payment, completes in < 3 minutes — § "...Functional Requirements" (line 465) — see Open questions for a tension with the 5-minute completion assumption.
- [ ] **AC-08-35** Payment success rate is 95%+ (Stripe reliability) — § "...Functional Requirements" (line 466)
- [ ] **AC-08-36** Shareable links work across all devices — § "...Functional Requirements" (line 467)
- [ ] **AC-08-37** Form submissions are stored correctly with all data — § "...Functional Requirements" (line 468)
- [ ] **AC-08-38** The user conversion flow works seamlessly — § "...Functional Requirements" (line 469)
- [ ] **AC-08-39** Forms load in < 2 seconds — § "Success Criteria — Non-Functional Requirements" (line 472)
- [ ] **AC-08-40** Forms are mobile-friendly (responsive design) — § "...Non-Functional Requirements" (line 473)
- [ ] **AC-08-41** Forms meet WCAG 2.1 Level AA accessibility — § "...Non-Functional Requirements" (line 474)
- [ ] **AC-08-42** Forms are served over HTTPS, and payment data is never stored on the platform — § "...Non-Functional Requirements" (line 475)
- [ ] **AC-08-43** Trainer adoption of camp/evaluation forms reaches 70%+ within the first 3 months — § "Success Criteria — Business Requirements" (line 478)
- [ ] **AC-08-44** Camp-participant-to-full-user conversion rate reaches 30%+ — § "...Business Requirements" (line 479; also "Business Value — Success Metrics," line 40)
- [ ] **AC-08-45** Camp revenue accounts for 20%+ of platform transactions — § "...Business Requirements" (line 480) — see Open questions for a similarly-shaped but distinct 20%+ metric elsewhere in the epic.
- [ ] **AC-08-46** Zero payment processing errors — § "...Business Requirements" (line 481; also "Business Value — Success Metrics," line 42, which scopes this specifically to "camp purchases")

**Final count: 46 acceptance criteria (AC-08-1 .. AC-08-46)**, covering all
6 user stories (US-08.01–US-08.06 — matching the epic's own closing count,
"Total User Stories: 6," line 544) and the epic-level "Success Criteria"
(Functional, Non-Functional, and Business Requirements), with every
per-story "Business Rules" item folded into its related criterion instead
of being dropped or listed twice.

## Business rules

Restated from "Business Rules" (line 354):

- [ ] **BR-08-1** Camps and Evaluations start from pre-loaded templates — `Epic-08_Forms_Registration_SPEC.md` § "Business Rules — Form Creation" (line 357)
- [ ] **BR-08-2** Form fields are limited to four types: Text, Email, Dropdown, Multi-select — § "...Form Creation" (line 358)
- [ ] **BR-08-3** Camps have capacity limits; Evaluations do not — § "...Form Creation" (line 359)
- [ ] **BR-08-4** Pricing is optional for both camps and evaluations — § "...Form Creation" (line 360)
- [ ] **BR-08-5** Camps can be turned on and off; Evaluations are always on — § "...Form Creation" (line 361)
- [ ] **BR-08-6** Forms are publicly accessible via the shareable link; no login is required — § "...Form Submission" (line 364)
- [ ] **BR-08-7** If a camp is full, the form shows a "Camp Full" message and does not allow submission — § "...Form Submission" (line 365)
- [ ] **BR-08-8** For paid camps, payment must complete before registration is confirmed — § "...Form Submission" (line 366)
- [ ] **BR-08-9** The email field must be a valid format — § "...Form Submission" (line 367)
- [ ] **BR-08-10** The same email address cannot submit to the same form twice — § "...Form Submission" (line 368)
- [ ] **BR-08-11** All payments are processed via Stripe Checkout; there is no custom payment form — § "...Payment Processing" (line 371)
- [ ] **BR-08-12** The platform's application fee (5%, Dale's platform cut) is applied to camp revenue — § "...Payment Processing" (line 372)
- [ ] **BR-08-13** Refunds are handled manually via the Stripe Dashboard for MVP; there is no automated refund flow — § "...Payment Processing" (line 373)
- [ ] **BR-08-14** A Stripe webhook confirms payment, which marks the registration as "Paid" — § "...Payment Processing" (line 374)
- [ ] **BR-08-15** Account creation after camp registration is optional — § "...User Conversion" (line 377)
- [ ] **BR-08-16** Form submission data pre-fills the registration form — § "...User Conversion" (line 378)
- [ ] **BR-08-17** Converted users are automatically assigned to the trainer — § "...User Conversion" (line 379)
- [ ] **BR-08-18** If the submitter's email already exists in the system, they are prompted to log in instead of creating a duplicate account — § "...User Conversion" (line 380)
- [ ] **BR-08-19** Trainers can only view and edit their own forms — § "...Trainer Permissions" (line 383)
- [ ] **BR-08-20** Coaches cannot create or manage forms; this is a trainer-only feature — § "...Trainer Permissions" (line 384)
- [ ] **BR-08-21** Super Admin can view all forms across all trainers, via impersonation mode — § "...Trainer Permissions" (line 385)

## Data requirements

Restated from "Data Model" (line 311) — entities, the fields the epic
names, and the relationships it names. The epic presents these as
schema-style code blocks (`form_id (UUID, primary key)`, etc.); no schema,
keys, or types beyond what it already states are proposed here.

- **Form** (a camp or evaluation): a form type of either "camp" or "evaluation," a form name (e.g., "Summer Skills Camp"), a description, a price (nullable — $0 means free), a capacity limit (nullable; the epic notes this applies to camps only), an active/inactive flag (the epic notes this applies to camps only), the form's field definitions stored as a JSON array (see Form Field Definition below), a created and an updated timestamp, and a unique shareable link. Relationship: belongs to a trainer. (lines 313-327)
- **Form Submission**: submission data stored as JSON, holding all of the submitter's responses to the form's fields; a payment status of "free," "paid," or "pending"; a Stripe payment identifier (nullable); a submitted-at timestamp; a converted-to-user flag (defaults to not converted); and, if converted, a reference to the resulting user account (nullable). Relationship: belongs to a Form. (lines 329-339)
- **Form Field Definition** (the JSON structure the epic gives for each entry in a form's field-definitions array): a field identifier, a field type of "text," "email," "dropdown," or "multiselect," a label (e.g., "Participant Name"), a required flag, and — for dropdown or multi-select fields only — a list of selectable options. (lines 341-350)

**Note on Camp vs. Evaluation** (from "Data Model — Form Entity," lines 317,
321-322): the epic gives Camps and Evaluations one shared Form structure,
distinguished by a form-type value ("camp" or "evaluation"); capacity limit
and the active/inactive flag are both described as applying "only for
camps," implying Evaluations carry those fields as always empty/not
applicable rather than the epic describing a separate Evaluation structure.
This spec does not resolve how that should be modeled.

## Assumptions (from the epic)

Not an acceptance-criteria source, but restated in full from "Assumptions"
(line 147) so it is not lost.

**Business Assumptions** (line 149)
- Most camps have pricing and are revenue-generating; evaluations are often free and used for lead generation (lines 150-151)
- External link sharing is the primary distribution method (line 152)
- Conversion to a user account is optional, not forced (line 153)
- Camp participants expect a simple, fast registration process (line 154)

**Technical Assumptions** (line 156)
- Form data is stored in the platform database (line 157)
- Stripe Checkout handles payment processing rather than a custom payment form (line 158)
- Form links are publicly accessible; no login is required (line 159)
- Capacity limits are enforced at submission time, not via a live countdown (line 160)
- Form validation is basic: required fields and email format only (line 161)

**UX Assumptions** (line 163)
- Forms are mobile-friendly, since mobile is the primary device for submissions (line 164)
- Forms are one page, not multi-step (line 165)
- Form completion takes < 5 minutes (line 166) — see Open questions for a tension with the 3-minute Success Criteria target.
- Payment steps have clear progress indicators (line 167)

## Edge cases

| Case | Expected |
|---|---|
| Camp registration is attempted after capacity is reached | Form shows "Camp Full" and does not allow submission — US-08.03 "Business Rules" (line 236); Business Rules "Form Submission" (line 365) |
| The same email address submits the same form a second time | Blocked — "same email cannot submit to same form twice" — Business Rules "Form Submission" (line 368) |
| A paid camp/evaluation submission's payment does not complete | Registration is not confirmed; a pending payment does not count as a registration and is excluded from the trainer's participant view — US-08.03 (lines 230-231, 238); Business Rules "Form Submission" (line 366); US-08.04 "Business Rules" (line 260) |
| Stripe payment fails during checkout | Submitter sees a clear error message with a retry option and a support contact for payment issues — "Risks & Mitigations — Risk: Payment Processing Failures" (lines 494-499) |
| A bot or automated script submits the public form | Mitigated with honeypot fields, per-IP rate limiting on submissions, and requiring email confirmation before a submission is marked valid — "Risks & Mitigations — Risk: Form Spam Submissions" (lines 501-506) |
| Multiple submissions arrive at or near a camp's capacity limit at the same time | Capacity is enforced with an atomic, database-level check and optimistic locking on submissions, to prevent overselling — "Risks & Mitigations — Risk: Capacity Overselling" (lines 508-512) |
| A camp participant tries to convert to a user account using an email that already has a platform account | Prompted to log in instead of creating a duplicate account — US-08.05 "Business Rules" (lines 284-285); Business Rules "User Conversion" (line 380) |
| A camp participant declines to create an account immediately | Receives an email with a ShareLink to register later; conversion is never forced — US-08.05 "Alternative Flow" (lines 280-281); Business Rules "User Conversion" (line 377) |
| Trainer tries to reduce a camp's capacity limit below its current registration count | Not allowed — US-08.06 "Business Rules" (line 305) |
| Trainer tries to delete a form that has paid registrations | Blocked; registrations must be refunded via Stripe first — US-08.06 "Business Rules" (line 306) |
| Trainer disables a camp | The camp's shareable link shows a "Registration Closed" message instead of the form — US-08.06 "Business Rules" (line 307) |
| Trainer edits a form that already has submissions | A warning names how many participants are already registered — US-08.06 "Business Rules" (line 304) |

The "Risk: Low Conversion Rate" entry in "Risks & Mitigations" (lines
487-492) does not state a specific system behavior the way the other three
risks do; its mitigations are discussed in Open questions instead of
guessed here as an edge case.

## Out of MVP scope

Verbatim from "Out of Scope (Post-MVP)" (line 102):

**Phase 2 Deferrals** (line 104)
- Complex form fields (file uploads, images, signature pads, tables)
- Conditional logic (show/hide fields based on answers)
- Multi-page forms (all fields on one page for MVP)
- Form analytics (completion rates, drop-off points)
- Email reminders for incomplete registrations
- Refund processing for camps (handled manually via Stripe for MVP)
- Waitlist for camps (show "Full" only)
- Early bird pricing (single price only)
- Group discounts (family/team pricing)
- Recurring evaluations (evaluation requests from players)
- Evaluation scheduling (time slot selection for in-person evaluations)
- Video upload in evaluation forms (YouTube link only if needed)
- Form versioning (edit history)
- Form duplication/templates beyond pre-loaded
- Integration with external form tools (Typeform, Google Forms)

**Simplifications** (line 121)
- Basic form builder (not drag-and-drop)
- Template-based approach (pre-loaded Camps & Evaluations templates)
- Simple field types only
- No complex validation rules

## Open questions

### From the epic's "Open Questions" section (line 436, verbatim)

| ID | Question | Priority | Recommendation | Impact |
|:---|:---|:---:|:---|:---|
| Q-08.01 | What's the maximum number of custom fields per form? | P2 | 20 fields max (prevents overly complex forms) | User experience (long forms = lower completion rates) |
| Q-08.02 | Should trainers see completion rates / drop-off points? | P2 | Defer to Phase 2 (nice-to-have, not essential for MVP) | Development time (+1 week if included) |
| Q-08.03 | Should evaluations allow players to select appointment times? | P2 | Defer to Phase 2 (adds significant complexity) | +2-3 weeks if included (requires calendar integration) |
| Q-08.04 | What's the refund policy for camps? | P2 | Manual refunds via Stripe for MVP (no automated policy) | Minimal (trainer handles edge cases manually) |

Q-08.03 additionally carries context not captured by the table above:
"Dale mentioned this as optional enhancement" (line 450).

### Analyst-raised

- **(analyst-raised)** Dependency-order contradiction. Epic-08's own §5
  "Dependencies — Required Before This Epic" states plainly that this epic
  requires Epic-01 and Epic-05 to already exist (lines 132-133), and
  `Epic_Areas_Plan.md`'s own caption under the dependency diagram agrees:
  "Epic-08 depends on Epic-01 (user conversion) and Epic-05 (payments)"
  (`Epic_Areas_Plan.md`, line 36). But the diagram itself, immediately
  above that caption, draws Epic-08 as a peer of Epic-02/Epic-03/Epic-04
  that feeds *into* Epic-05 (`Epic_Areas_Plan.md`, lines 15-32) — i.e.,
  placed *before* Epic-05, the opposite of what both the epic and the
  plan's own caption say. This spec follows the epic's explicit dependency
  statement (Epic-08 depends on Epic-05, not the reverse); the diagram
  needs correcting.
- **(analyst-raised)** Epic-08 is missing from MVP delivery planning.
  `Epic_Areas_Plan.md`'s "MVP Prioritization" lists four delivery phases,
  Phase 1A through Phase 1D (lines 492-519), covering Epics 01, 02, 05, 03,
  06, 04, and 07 — Epic-08 appears in none of them, despite the same
  document's own Epic-08 write-up stating "Camps IN MVP as separate
  Epic-08" as a "Key Decision" (`Epic_Areas_Plan.md`, line 485), and
  despite Epic-08's own file carrying no "Post-MVP" or "Phase 2" framing at
  the epic level (only specific features within it are deferred, per Out
  of MVP scope above). Which phase Epic-08 belongs to, or whether it
  constitutes its own phase, is not stated anywhere.
- **(analyst-raised)** Epic number "08" is reused for a different,
  unrelated epic. `Epic_Areas_Plan.md`'s "Post-MVP Epics (Phase 2+)"
  section lists "Epic 08: Advanced Communication (Phase 2)" — in-app
  messaging, SMS notifications, push notifications, video calls
  (`Epic_Areas_Plan.md`, lines 525-529) — reusing the number this spec's
  Epic-08 (Forms & Registration) already occupies as an MVP epic. This is a
  numbering collision in the planning document, not a scope change to
  either epic; Advanced Communication needs a different number.
- **(analyst-raised)** The "Risk: Low Conversion Rate" mitigation list
  includes "Offer incentive (e.g., 'Free token' for account creation)"
  (`Epic-08_Forms_Registration_SPEC.md` § "Risks & Mitigations — Risk: Low
  Conversion Rate," line 492) as an example, not a commitment. No token
  incentive for account conversion appears anywhere in Epic-08's own In
  Scope (MVP) list, its Business Rules, or its Data Model, and tokens
  themselves are an Epic-05 concept. Whether an incentive is actually
  planned for MVP, and if so whether it is Epic-08's or Epic-05's
  responsibility to implement, is unresolved.
- **(analyst-raised)** US-08.04's own Acceptance Criteria and Business
  Rules disagree with each other. The Acceptance Criteria list "Pending" as
  one of three filterable Payment Status values alongside Paid and Free
  (line 253), but the same story's Business Rules state "Only paid/free
  registrations shown (pending payments excluded)" (line 260) — restated in
  AC-08-18 above. If pending submissions never appear in the list, a
  "Pending" filter option has nothing to filter; unresolved whether pending
  registrations should actually be visible (e.g., grayed out, so trainers
  can follow up) or the filter option is left over from an earlier draft.
- **(analyst-raised)** Two differently-worded targets both describe the
  same submitter action without being reconciled. "UX Assumptions" states
  "Form completion takes < 5 minutes" (line 166), while "Success Criteria —
  Functional Requirements" states "Form submission completes in < 3 minutes
  (including payment)" (line 465). Both describe the same end-to-end
  submitter action (filling and submitting the form, including payment); it
  is unclear whether 3 minutes supersedes the assumption's 5 minutes as the
  real target, or whether they describe different scopes (e.g., form-only
  vs. form-plus-payment).
- **(analyst-raised)** Two different "20%+" business metrics are stated for
  camps without being reconciled: "Business Value — Success Metrics" states
  "Camp registrations account for 20%+ of new player acquisitions" (line
  41), while "Success Criteria — Business Requirements" states "Camp
  revenue accounts for 20%+ of platform transactions" (line 480, restated
  as AC-08-45). These measure different things (an acquisition count vs. a
  revenue share) and could both be genuine, independent targets that happen
  to share a round number, or one could be a restatement of the other with
  the subject changed in error. Not resolved here.
- **(analyst-raised)** The Epic-07 feature toggle that gates Camps
  (US-07.05) is cited by this epic only in passing — "Feature Toggle:
  'Camps' can be enabled/disabled per trainer" (§ "Integration Points —
  Epic-07: Super Admin & System Management," line 403) — and this epic
  never states whether that toggle controls this epic's camp *forms*, or
  Epic-02's camp *events*, or both. The Epic-07 spec produced in this same
  run flags the identical ambiguity from its own side
  (`requirements-analyst-epic-07-super-admin-spec.md`, Open questions,
  analyst-raised, "Minor inconsistency in which epic the 'Camps' feature
  toggle affects"). This spec does not resolve it here either.
- **(analyst-raised, methodology note)** Two headings in this file collide
  with bold sub-labels used earlier in the file for a different purpose.
  "Business Value" is both a bold sub-label inside §1 "Description" (line
  20, listing Lead Generation/Revenue/Conversion/Flexibility) and the full
  heading of §2 (line 28, with its own "Problem Statement" and "Success
  Metrics" sub-labels). "Integration Points" is both a bold sub-label
  inside §5 "Dependencies" (line 140, three bullets) and the full heading
  of §10 (line 389, with per-epic detail). Every back-reference in this
  spec cites heading text plus line number for this reason.
