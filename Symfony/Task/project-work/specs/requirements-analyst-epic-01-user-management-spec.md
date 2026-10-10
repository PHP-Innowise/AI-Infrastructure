# Spec: Epic-01 — User Management & Authentication

Source: `Task/Epics/Epic-01_User_Management_Authentication_SPEC.md` (read-only client material)

> Citation note: this epic file numbers three sections twice (§10, §11, §12 each
> appear on two different headings — see Open questions). Every back-reference
> below therefore cites heading **text** plus a line number, never a bare
> section number.

## Problem

Epic-01 is the platform's foundation: every other epic depends on user
accounts, roles, and trainer organizations already existing, so without a
working multi-role authentication and access-control system the platform
cannot function ("Description", line 7; "Business Value" bullets, line 17;
"Dependencies — Blocks These Epics", lines 116-121). The platform serves four
distinct user types — a single Super Admin operator, trainers running their
own training businesses, coaches who each deliver sessions under one trainer
at a time, and players/parents attending training — each needing different
permissions and workflows; a generic one-size-fits-all user system would
create security risks, confusing UX, and operational overhead ("Description",
lines 7-9; "Business Value — Problem Statement", line 24). This epic must
therefore give Super Admin the tools to onboard and support trainers
(including impersonation for troubleshooting), let trainers run and brand
their own organization autonomously while staying fully isolated from other
trainers' data, let coaches work cleanly under exactly one trainer, and let
players/parents self-serve registration, multi-trainer participation, child
profiles, and purchase approval ("Description — Business Value", lines 11-16;
"Business Value — Success Metrics", lines 27-32).

## User scenarios

1. **Super Admin** creates a new trainer account so that the trainer can
   start using the platform to run their business.
   Path: Super Admin opens the Users tool, creates a user with role
   "Trainer", enters business name/trainer name/email/phone; the system
   emails the new trainer their credentials or a setup link; the trainer
   logs in and lands on their trainer dashboard.
   Source: US-01.01 "Super Admin Creates Trainer Account" (line 144)

2. **Player or Parent** registers for training via a link from their trainer
   so that they can join the program and see available sessions.
   Path: Player/Parent clicks the trainer's ShareLink, registers (or logs in
   if they already have an account), and is auto-associated with that
   trainer; a returning player/parent with children who clicks a second
   trainer's link chooses which family members join that trainer, and each
   trainer relationship becomes its own separated, isolated context.
   Source: US-01.02 "Player Registers via ShareLink" (line 167)

3. **Parent** creates profiles for their children so that they can manage
   training for their whole family from one account.
   Path: Parent opens Player Profiles, clicks "+ Add Child", enters name/
   age/gender, and is prompted to associate the child with their trainer(s);
   the child profile is linked to the parent account and the parent can
   switch between children in the UI.
   Source: US-01.03 "Parent Creates Child Profile" (line 206)

4. **Parent** adds or removes their children from trainers so that they can
   control which programs each child participates in.
   Path: Parent opens the "Family"/"Player Profiles" section, adds a child
   to a trainer via ShareLink or by picking from "My Trainers", or removes a
   child from a trainer after a warning that this cancels the child's
   upcoming RSVPs with that trainer.
   Source: US-01.04 "Parent Manages Child-Trainer Associations" (line 238)

5. **Parent** controls their child's spending so that they can manage family
   finances and verify training choices.
   Path: Child selects a paid event; checkout is held as "Pending Parent
   Approval"; the parent is notified by email and in-app, reviews the
   request, and approves (payment processes, child sees "Confirmed") or
   denies it; an unanswered request auto-expires after 48 hours.
   Source: US-01.05 "Child Purchase Requires Parent Approval" (line 303)

6. **Parent** gives their child a limited login so that the child can view
   their training but cannot make unauthorized changes.
   Path: Logged-in child browses events, RSVPs, and views content within a
   fixed, reduced permission set; if the child clicks a new trainer's
   ShareLink, registration is blocked and the parent is emailed to complete
   it instead.
   Source: US-01.06 "Child Login with Constraints" (line 335)

7. **Super Admin** views the platform as any user so that they can
   troubleshoot issues and provide support.
   Path: Super Admin finds a user in the Users tool, clicks "Impersonate",
   confirms, and sees the platform exactly as that user with a persistent
   "Viewing as" banner, until clicking "Exit Impersonation".
   Source: US-01.07 "Super Admin Impersonates User" (line 383)

8. **Trainer** invites a coach to work with their organization so that they
   can delegate session delivery and expand their program.
   Path: Trainer opens the Coaches section, invites a coach by email, the
   coach receives a one-time ShareLink, registers, and appears in the
   trainer's Coaches list once accepted.
   Source: US-01.08 "Trainer Invites Coach" (line 407)

9. **Player or Parent** sets their/their child's availability preferences so
   that trainers can schedule events when they are available to attend.
   Path: Player/Parent opens "Availability"/"My Times", sets available time
   ranges per day (separately per child if applicable), and saves; trainers
   see this availability when planning events.
   Source: US-01.09 "Player/Parent Sets Availability" (line 431)

10. **Coach** defines their weekly availability so that trainers schedule
    them for sessions they can attend.
    Path: Coach opens "My Times", sets a recurring weekly schedule (multiple
    slots per day allowed) and saves; if a trainer assigns them outside that
    schedule, the trainer sees a warning and must override with a reason.
    Source: US-01.10 "Coach Sets My Times (Availability)" (line 454)

11. **Any user** updates their profile information so that their details are
    current and accurate.
    Path: User opens "Profile"/"Account Settings", edits the fields open to
    them (name, phone, photo, plus role-specific fields), and saves; email,
    role, and (for players) skill level stay read-only.
    Source: US-01.11 "User Edits Own Profile" (line 476)

12. **Super Admin** deactivates a user account so that the user cannot log
    in but their history is preserved.
    Path: Super Admin clicks "Deactivate" on a user in the Users tool,
    confirms, and the user is blocked from login while all their historical
    records stay visible; Super Admin can reactivate the account later.
    Source: US-01.12 "Super Admin Deactivates User" (line 507)

13. **Super Admin** permanently deletes a user's personal information so
    that the platform complies with GDPR/privacy requests.
    Path: Super Admin clicks "Delete" on a user, confirms an explicit
    "cannot be undone" warning, and the user's personal fields are
    anonymized while historical records persist under "Deleted User".
    Source: US-01.13 "Super Admin Deletes User (GDPR Compliance)" (line 526)

14. **Trainer** customizes their portal's appearance so that their brand
    identity is reflected to players and coaches.
    Path: Trainer opens Portal Settings/Branding, uploads a logo and picks a
    primary color with a live preview, and saves so the branding applies
    immediately across their organization's portal.
    Source: US-01.14 "Trainer Customizes Portal Branding" (line 551)

## Acceptance criteria

Every checkbox below is renumbered sequentially as AC-01-1 .. AC-01-78 from
the epic's per-story "Acceptance Criteria" lists, its equivalent
role-permission lists in US-01.06, one criterion derived from the epic-level
"In Scope (MVP)" scope note on Camp-to-User Conversion, and the epic-level
"Acceptance Criteria (Epic-Level)" section. Where an epic-level criterion
restates a story criterion, the story version is kept and both origins are
cited.

**US-01.01 — Super Admin Creates Trainer Account** (line 144)
- [ ] **AC-01-1** Super Admin can create a trainer account from the Users tool by selecting the "Trainer" role — `Epic-01_User_Management_Authentication_SPEC.md` § "US-01.01: Super Admin Creates Trainer Account" (line 151; also epic-level AC § "Acceptance Criteria (Epic-Level)", line 948)
- [ ] **AC-01-2** Trainer creation captures business name, trainer name, email, and phone — § "US-01.01..." (line 152)
- [ ] **AC-01-3** On creation, the system generates a temporary password or sends an invite email with a setup link — § "US-01.01..." (line 153)
- [ ] **AC-01-4** The new trainer receives an email with login credentials or setup instructions — § "US-01.01..." (line 154)
- [ ] **AC-01-5** The new trainer can log in and access the trainer dashboard — § "US-01.01..." (line 155)
- [ ] **AC-01-6** The new trainer account appears in the Users list with status "Active" — § "US-01.01..." (line 156)
- [ ] **AC-01-7** Email must be unique and required fields are enforced on trainer creation — § "US-01.01..." (line 157)
- [ ] **AC-01-8** A duplicate email shows a clear error message — § "US-01.01..." (line 158)

**US-01.02 — Player Registers via ShareLink** (line 167)
- [ ] **AC-01-9** Clicking a trainer's ShareLink while logged out redirects to the registration/login page; clicking it while already logged in creates an instant association and redirects to the trainer's events — § "US-01.02: Player Registers via ShareLink" (lines 174-175, 181)
- [ ] **AC-01-10** The registration form captures name, email, password, parent phone, and player name/age/gender — § "US-01.02..." (line 176)
- [ ] **AC-01-11** After registration, the player is auto-associated with the trainer who sent the link, a player profile is created in that trainer's CRM, and the player can view the trainer's events and content — § "US-01.02..." (lines 177-179; also epic-level AC, line 957)
- [ ] **AC-01-12** A confirmation email is sent to the player/parent after registration — § "US-01.02..." (line 180)
- [ ] **AC-01-13** A player with an existing account who clicks a different trainer's ShareLink is associated with the second trainer without creating a duplicate account — § "US-01.02..." (lines 184-185; also epic-level AC, line 958)
- [ ] **AC-01-14** If the account is a parent with children, clicking a new trainer's ShareLink shows a selection prompt ("Who will train with [New Trainer]?") listing the parent ("Me") and all children; only the family members selected are associated with the new trainer — § "US-01.02..." (lines 186-190)
- [ ] **AC-01-15** Multi-trainer players see separated views: they switch between trainer contexts like switching accounts, each context shows fully isolated data (calendar, tokens, content, reservations) with no combined view, a context switcher is available in navigation, and the current context persists across the session — § "US-01.02..." (lines 191-196)

**US-01.03 — Parent Creates Child Profile** (line 206)
- [ ] **AC-01-16** Parent can add a child profile from the Player Profiles page, entering child name, age, gender, and optionally school/photo, marked "Child" (vs "Self") — § "US-01.03: Parent Creates Child Profile" (lines 213-215; also epic-level AC, line 959)
- [ ] **AC-01-17** Trainer association on child creation follows the parent's trainer count: with one trainer the parent is prompted "Will [Child] also train with [Trainer]?" (Yes/No); with multiple trainers a selection checklist is shown; a "Yes" or a selection associates the child with those trainers, otherwise the child profile is created without a trainer association — § "US-01.03..." (lines 216-220)
- [ ] **AC-01-18** The child profile is linked to the parent account, and the parent can switch between children via a context selector in the UI — § "US-01.03..." (lines 221-222)
- [ ] **AC-01-19** Each child has a separate training calendar, RSVP status, attendance record, and availability preferences per trainer — § "US-01.03..." (line 223)
- [ ] **AC-01-20** A child can optionally have a separate login, sharing the parent's contact info and requiring parent approval for purchases — § "US-01.03..." (line 224)
- [ ] **AC-01-21** Child profile creation requires name, age, and gender; age must be 1-18 years (adults use their own accounts); the system warns on a duplicate check if a similar name/age profile already exists — § "US-01.03..." (lines 227-229)

**US-01.04 — Parent Manages Child-Trainer Associations** (line 238)
- [ ] **AC-01-22** Parent can navigate to a "Family"/"Player Profiles" section and view every child with their trainer associations (name, age, associated trainers with dates) — § "US-01.04: Parent Manages Child-Trainer Associations" (lines 245-247)
- [ ] **AC-01-23** Parent can add a child to a trainer either by entering a ShareLink manually or by selecting from "My Trainers"; after confirmation the child is associated with that trainer and can see the trainer's events/content — § "US-01.04..." (lines 250-254)
- [ ] **AC-01-24** Parent can remove a child from a trainer; the system warns that this cancels all the child's upcoming RSVPs with that trainer, and on confirmation the child is disassociated, the trainer no longer sees the child in their roster, and the child's data with that trainer is soft-deleted with history preserved — § "US-01.04..." (lines 257-262)

**US-01.05 — Child Purchase Requires Parent Approval** (line 303)
- [ ] **AC-01-25** When a child selects an event requiring a USD payment, checkout sets the request to "Pending Parent Approval" and the parent is notified by email and in-app — § "US-01.05: Child Purchase Requires Parent Approval" (lines 310-312; also epic-level AC, line 960)
- [ ] **AC-01-26** The parent reviews pending purchase requests (Payments/Reservations) and can approve (payment processed, child registered, status becomes "Confirmed"), deny (child notified), or request more information — § "US-01.05..." (lines 313-316)
- [ ] **AC-01-27** Parent has a per-child setting "Allow token spending without approval" (default OFF); when OFF, token purchases follow the same approval workflow as USD payments; when ON, the child's token purchase and event registration are processed immediately and the parent receives only an informational notification — § "US-01.05..." (lines 319-324)
- [ ] **AC-01-28** Parent can change the token-approval setting for a child at any time from the child's profile settings — § "US-01.05..." (line 325)

**US-01.06 — Child Login with Constraints** (line 335)
- [ ] **AC-01-29** While logged in, a child can: browse eligible events (view-only), RSVP and cancel RSVP (both require parent approval), view purchased content, view their own training progress, submit feedback requests, update basic profile info, view tokens (view-only), and switch between trainer contexts if they train with multiple trainers — § "US-01.06: Child Login with Constraints" § "Child CAN Do" (lines 342-350)
- [ ] **AC-01-30** A child cannot: add new trainers (ShareLink registration is blocked), add/remove payment methods, purchase tokens, complete purchases without parent approval, delete their account, change trainer associations, or view the parent's training data — § "US-01.06..." § "Child CANNOT Do" (lines 353-359)
- [ ] **AC-01-31** When a logged-in child clicks a trainer ShareLink, the system detects the child account, tells the child to ask their parent to register them, emails the parent a "Review Registration" call to action, and does not associate the child with the new trainer until the parent completes registration — § "US-01.06..." § "ShareLink Blocking Flow" (lines 362-369)
- [ ] **AC-01-32** If a child trains with multiple trainers, a context selector lists only the child's own trainer contexts as a simple trainer list, with no parent data and no "Me" section — § "US-01.06..." § "Context Switching for Children" (lines 372-374)

**US-01.07 — Super Admin Impersonates User** (line 383)
- [ ] **AC-01-33** Super Admin can impersonate a user from the Users tool via an "Impersonate" button and a confirmation modal naming the target user and role — § "US-01.07: Super Admin Impersonates User" (lines 390-391; also epic-level AC, line 952)
- [ ] **AC-01-34** After confirming impersonation, the portal switches to the impersonated user's view with a sticky, color-coded top banner ("Viewing as [User Name] | Exit Impersonation"), and all navigation, permissions, and data exactly match the impersonated user — § "US-01.07..." (lines 392-395)
- [ ] **AC-01-35** Clicking "Exit Impersonation" returns the Super Admin to their own view — § "US-01.07..." (line 396)
- [ ] **AC-01-36** Every impersonation session is logged (who impersonated whom, start time, end time, duration), all actions taken during the session are logged with the admin's ID context, and an "Impersonation History" audit report is available — § "US-01.07..." (lines 397, 401, 403; also epic-level AC, line 953)
- [ ] **AC-01-37** Super Admin cannot impersonate another Super Admin account; the system returns a validation error — § "US-01.07..." (line 400)
- [ ] **AC-01-38** An impersonation session expires automatically after 1 hour if not explicitly exited — § "US-01.07..." (line 402)

**US-01.08 — Trainer Invites Coach** (line 407)
- [ ] **AC-01-39** Trainer invites a coach from the Coaches section by entering the coach's email (name/message optional); the system generates a unique, one-time-use ShareLink with a 7-day expiry and emails the invitation — § "US-01.08: Trainer Invites Coach" (lines 414-417; also epic-level AC, line 965)
- [ ] **AC-01-40** The invited coach registers/logs in via the link, is associated with the inviting trainer with status "Pending" or "Active", and appears in the trainer's Coaches list once accepted; the trainer can view invitation status (Pending, Accepted, Expired) — § "US-01.08..." (lines 418-421; also epic-level AC, line 966)
- [ ] **AC-01-41** A coach can only be active under one trainer at a time; email is required on invite, and if the coach already exists and is active elsewhere the system shows an error rather than allowing a second active trainer — § "US-01.08..." (lines 422, 425-426)
- [ ] **AC-01-42** An expired invitation link shows a clear message with an option to resend the invitation — § "US-01.08..." (line 427)

**US-01.09 — Player/Parent Sets Availability** (line 431)
- [ ] **AC-01-43** From navigation, a player/parent can open an "Availability"/"My Times" grid showing days of the week and time slots, toggle each day available/not-available or set custom time ranges, save, and receive a save confirmation — § "US-01.09: Player/Parent Sets Availability" (lines 438-442, 444; also epic-level AC, line 961)
- [ ] **AC-01-44** A parent can set separate availability per child via a profile switcher — § "US-01.09..." (line 443)
- [ ] **AC-01-45** Trainers can see a player-availability indicator during event creation/in the CRM, filter players by availability at a selected day/time, see a per-player "Best Times" summary on the player card, and use availability to suggest session times matching the most players — § "US-01.09..." § "Trainer View" (lines 447-450; also epic-level AC, line 974)

**US-01.10 — Coach Sets My Times (Availability)** (line 454)
- [ ] **AC-01-46** Coach can set a recurring weekly availability schedule ("My Times") by weekday and time range, with multiple time slots allowed per day, and save it — § "US-01.10: Coach Sets My Times (Availability)" (lines 461-465; also epic-level AC, line 967)
- [ ] **AC-01-47** When a trainer assigns a coach to an event at a time conflicting with the coach's stated availability, the system warns the trainer, requires a text reason to override, and logs the override (event, coach, reason, overriding trainer); the coach sees the assignment (not blocked) and can accept or request a change — § "US-01.10..." § "Trainer Assignment Flow" (lines 468-472; also epic-level AC, lines 968, 969, 975)

**US-01.11 — User Edits Own Profile** (line 476)
- [ ] **AC-01-48** Any user can open "Profile"/"Account Settings" and edit common fields (first/last name, phone number, profile photo, and optionally school/coach bio/player jersey number), while email, role, player skill level, and account-created date stay read-only — § "US-01.11: User Edits Own Profile" (lines 483-493; also epic-level AC, line 962)
- [ ] **AC-01-49** Saving profile changes persists them with a confirmation message; an uploaded profile photo is stored in file storage with a generated thumbnail and updated photo URL — § "US-01.11..." (lines 494-495)
- [ ] **AC-01-50** Profile edits validate phone number format and enforce required fields — § "US-01.11..." (line 496)
- [ ] **AC-01-51** Role-specific profile fields apply on top of the common set: Player (school, jersey number, photo), Parent (emergency contact info if they have children), Coach (bio, credentials, certifications, public-profile checkbox), Trainer (business name, organization details), Super Admin (admin-specific settings such as email notifications) — § "US-01.11..." § "Role-Specific Fields" (lines 498-503; also epic-level AC, line 970)

**US-01.12 — Super Admin Deactivates User** (line 507)
- [ ] **AC-01-52** Super Admin can deactivate a user from the Users tool; a confirmation modal states the user will not be able to log in and that historical data is preserved, and on confirmation the user's status becomes "Inactive" with login blocked and an "Account deactivated" message — § "US-01.12: Super Admin Deactivates User" (lines 514-517; also epic-level AC, line 950)
- [ ] **AC-01-53** A deactivated user still appears in historical analytics (attendance, payments, referrals), past event rosters, and CRM records (shown grayed out / marked "Inactive") — § "US-01.12..." (lines 518-521; also epic-level AC, lines 950, 981)
- [ ] **AC-01-54** Super Admin can reactivate a deactivated user, restoring status to "Active" and allowing login again — § "US-01.12..." (line 522)

**US-01.13 — Super Admin Deletes User (GDPR Compliance)** (line 526)
- [ ] **AC-01-55** Super Admin can delete a user from the Users tool; a warning confirmation modal states personal information will be removed, historical records will show "Deleted User", and the action cannot be undone — § "US-01.13: Super Admin Deletes User (GDPR Compliance)" (lines 533-534)
- [ ] **AC-01-56** On confirmed deletion, the user's personal fields are anonymized (name → "Deleted User", email → "deleted_[user_id]@example.com", phone → NULL, photo → default avatar, other personal identifiers → NULL) and status becomes "Deleted" — § "US-01.13..." (lines 535-540, 545; also epic-level AC, lines 951, 982)
- [ ] **AC-01-57** Historical records are preserved after deletion, showing "Deleted User" for event attendance and payments, with analytics totals (player counts, revenue, attendance rates) unchanged — § "US-01.13..." (lines 541-544)
- [ ] **AC-01-58** A deleted user cannot be reactivated; the anonymization is permanent — § "US-01.13..." (line 546)
- [ ] **AC-01-59** Deletion is logged with the original user ID, who deleted the user, when, and the reason, for legal compliance — § "US-01.13..." (line 547)

**US-01.14 — Trainer Customizes Portal Branding** (line 551)
- [ ] **AC-01-60** Trainer can navigate to "My Portal Settings"/"Branding" to upload a single logo image (PNG/JPG/SVG, max 2MB, recommended 200x200px with auto-resize if larger) with a pre-save preview; the logo displays in the trainer's portal header, visible to that trainer's players, coaches, and parents — § "US-01.14: Trainer Customizes Portal Branding" (lines 558-564, 574-576)
- [ ] **AC-01-61** Trainer can select a primary brand color via a color picker (hex code format), used for UI gradient/accent colors, with real-time preview and a reset-to-default option — § "US-01.14..." (lines 565-568, 577)
- [ ] **AC-01-62** Saving branding changes applies them immediately for all users in the trainer's organization — § "US-01.14..." (lines 570-571)
- [ ] **AC-01-63** MVP branding scope is limited to one logo and one primary color; multiple logos (e.g. light/dark mode), font customization, and full layout customization are explicitly excluded from MVP — § "US-01.14..." § "Scope for MVP" (lines 579-584)

**Cross-epic scope note (not a dedicated user story)**
- [ ] **AC-01-64** After a camp/evaluation form submission, the system prompts the submitter to create an account, pre-fills the registration form with the camp submission data for a seamless conversion (no re-entering information), and auto-assigns the new account to the trainer after creation; alternatively a ShareLink can be sent by email for later registration. This is in-scope for Epic-01 as an integration point with Epic-08's camp/evaluation forms — Epic-08's side of the integration is out of scope for this spec — `Epic-01_User_Management_Authentication_SPEC.md` § "In Scope (MVP)" (lines 56-61)

**Epic-level (from "Acceptance Criteria (Epic-Level)", line 935) — not already covered above**
- [ ] **AC-01-65** All 4 roles can log in with email/password — § "Acceptance Criteria (Epic-Level)" (line 940)
- [ ] **AC-01-66** Password reset flow works end-to-end — § "Acceptance Criteria (Epic-Level)" (line 941)
- [ ] **AC-01-67** Email verification sends and processes correctly — § "Acceptance Criteria (Epic-Level)" (line 942)
- [ ] **AC-01-68** Role-based access control is enforced so each role reaches its correct dashboard — § "Acceptance Criteria (Epic-Level)" (line 943)
- [ ] **AC-01-69** Session management works correctly: login, logout, and session expiry — § "Acceptance Criteria (Epic-Level)" (line 944)
- [ ] **AC-01-70** Users cannot access features outside their role's permissions — § "Acceptance Criteria (Epic-Level)" (line 945)
- [ ] **AC-01-71** Super Admin can edit any user account — § "Acceptance Criteria (Epic-Level)" (line 949)
- [ ] **AC-01-72** The Users tool shows all users with tool-specific search and filters (not a global search) — § "Acceptance Criteria (Epic-Level)" (line 954)
- [ ] **AC-01-73** Trainer can generate ShareLinks: static (unlimited-use, no-expiry) links for players and unique (one-time-use, 7-day-expiry) links for coaches — § "Acceptance Criteria (Epic-Level)" (line 973)
- [ ] **AC-01-74** Trainers see only their own organization's players and coaches; multi-tenancy is enforced so no trainer can see another trainer's data — § "Acceptance Criteria (Epic-Level)" (lines 976, 980)
- [ ] **AC-01-75** All user data is stored securely: passwords hashed, emails unique — § "Acceptance Criteria (Epic-Level)" (line 979)
- [ ] **AC-01-76** Audit logs capture all sensitive operations, including impersonation and user deletion — § "Acceptance Criteria (Epic-Level)" (line 983)
- [ ] **AC-01-77** Performance: dashboard loads in <2 seconds; a 10,000-user list loads in <3 seconds with pagination; profile edits save in <1 second; the platform supports 1,000 concurrent users — § "Acceptance Criteria (Epic-Level)" (lines 986-989)
- [ ] **AC-01-78** (Process gate, not product behavior) Epic-01 is considered complete only once the demo is approved, all P0 open questions are resolved, and the security review has passed if applicable — § "Acceptance Criteria (Epic-Level)" § "Approval" (lines 992-994)

**Final count: 78 acceptance criteria (AC-01-1 .. AC-01-78)**, covering all 14
user stories (US-01.01–US-01.14), the Camp-to-User Conversion scope note, and
the epic-level completion checklist, with 21 epic-level items folded into
their matching story criterion rather than duplicated.

## Business rules

Restated from "Business Rules & Logic" (line 696):

- [ ] **BR-01-1** Passwords must be securely hashed (industry-standard approach) — `Epic-01_User_Management_Authentication_SPEC.md` § "Business Rules & Logic — Authentication & Security" (line 701)
- [ ] **BR-01-2** Email must be unique across all users — § "...Authentication & Security" (line 702)
- [ ] **BR-01-3** Sessions should expire after a reasonable period of inactivity — § "...Authentication & Security" (line 703)
- [ ] **BR-01-4** Password reset links expire after 1 hour — § "...Authentication & Security" (line 704)
- [ ] **BR-01-5** Email verification links expire after 24 hours — § "...Authentication & Security" (line 705)
- [ ] **BR-01-6** Login attempts must be rate-limited to prevent brute-force attacks — § "...Authentication & Security" (line 706)
- [ ] **BR-01-7** Each user has exactly one role: Super Admin, Trainer, Coach, or Player/Parent — § "...Role-Based Access" (line 709)
- [ ] **BR-01-8** After login, users see the dashboard appropriate to their role and can only access permitted features, enforced on both frontend and backend — § "...Role-Based Access" (lines 710-712)
- [ ] **BR-01-9** Trainers can only see/manage their own organization's data — § "...Multi-Tenancy & Data Isolation" (line 715)
- [ ] **BR-01-10** Players can connect to multiple trainers; multi-trainer players see separated views, switching between isolated trainer contexts — § "...Multi-Tenancy & Data Isolation" (lines 716-717)
- [ ] **BR-01-11** Coaches can work for only ONE trainer at a time (strictly enforced) — § "...Multi-Tenancy & Data Isolation" (line 718)
- [ ] **BR-01-12** When a player connects to a new trainer, no duplicate account is created — only a new trainer association — § "...Multi-Tenancy & Data Isolation" (line 719)
- [ ] **BR-01-13** Only Super Admin can create trainer accounts (no self-registration), which ensures payment verification and quality control; the new trainer receives an invitation email with setup instructions — § "...Trainer Creation" (lines 722-724)
- [ ] **BR-01-14** Players register via a ShareLink from a trainer; the static link supports unlimited uses with no expiry. If the player already has an account they are auto-associated with the new trainer; if new, an account is created and associated — § "...Player Registration" (lines 727-730)
- [ ] **BR-01-15** Trainers invite coaches via a unique ShareLink (one-time use, 7-day expiry); a coach cannot be active under multiple trainers simultaneously, and the system validates the coach isn't already active elsewhere — § "...Coach Invitation" (lines 733-735)
- [ ] **BR-01-16** A parent can create multiple child profiles and owns all contact information for the family; each child has a separate training calendar, RSVP status, and Best Times — § "...Parent/Child Relationships" (lines 738, 740-741)
- [ ] **BR-01-17** ALL players under 18 require a parent-managed account — no independent accounts for minors — § "...Parent/Child Relationships" (line 739) — **contradicted by an open question; see Open questions (analyst-raised).**
- [ ] **BR-01-18** USD payments always require parent approval: child requests → "Pending Parent Approval" → parent notified → parent approves/denies → the request auto-expires after 48 hours with no action — § "...Child Purchase Approval Workflow" (lines 744-748)
- [ ] **BR-01-19** Token spending requires parent approval by default; a parent may enable "allow child to spend tokens without approval" per child — § "...Child Purchase Approval Workflow" (lines 749-752)
- [ ] **BR-01-20** Parent can add notes when approving or denying any request — § "...Child Purchase Approval Workflow" (line 753)
- [ ] **BR-01-21** Super Admin can impersonate any user except other Super Admins, with a clear visual indicator while impersonating — § "...Impersonation Rules" (lines 756-757)
- [ ] **BR-01-22** All impersonation sessions are logged (who, whom, start, end, duration) and expire after 1 hour; impersonation exists for support/troubleshooting — § "...Impersonation Rules" (lines 758-760)
- [ ] **BR-01-23** A deactivated user cannot log in, but all history is preserved (analytics, attendance, payments, referrals) and remains visible in historical records; Super Admin can reactivate — § "...User Deactivation (Soft Delete)" (lines 763-766)
- [ ] **BR-01-24** Deletion removes personal information (name, email, phone) while historical records are preserved but anonymized as "Deleted User"; analytics totals remain accurate; deletion is permanent and logged for compliance — § "...User Deletion (GDPR Compliance)" (lines 769-773)
- [ ] **BR-01-25** Players/Parents set preferred training times per player; coaches set a recurring weekly availability schedule; trainers can view and filter by availability for planning — availability drives scheduling suggestions, not hard restrictions — § "...Best Times / Availability" (lines 776-780)
- [ ] **BR-01-26** When a trainer assigns a coach to a conflicting time, the system warns and requires a reason to override; the override is logged (who, when, why), and the coach sees the assignment (not blocked) and can accept or request a change — § "...Coach Availability Conflicts" (lines 783-786)
- [ ] **BR-01-27** The system tracks which ShareLink was used for each registration, usage count per link, and usage timing (for Epic-06 analytics); static links are unlimited-use/no-expiry, unique coach links are one-time-use with 7-day expiry — § "...ShareLink Tracking" (lines 789-793)
- [ ] **BR-01-28** Email and phone number formats are validated; name and email are required for all users; children must be age 1-18; duplicate emails are prevented; ShareLink codes are unique — § "...Validation Rules" (lines 796-801)

## Data requirements

Restated from "Data Requirements" (line 593) — entities, the fields the epic
names, and the relationships it names. No schema, keys, or types are
proposed here.

**Note on Player/Parent** (from "In Scope (MVP)", line 64; "US-01.03...
Important Notes", line 232; BR-01-17): a parent account is itself treated as
a player account (a parent can train themselves), and a child can optionally
have its own login tied back to the parent. The epic treats Player and
Parent as one role slot that behaves as two distinct data subjects (self vs.
child, with different login/approval constraints) — this spec does not
resolve how that should be modeled.

- **User** (all roles): email (unique, login), password (hashed), role, status (Active/Inactive/Deleted), email verification status, password reset tokens (temporary), last login timestamp, creation/update timestamps. (line 597)
- **Profile** (common, all roles): first/last name, phone number, profile photo, school/organization (optional). Relationship: belongs to a User. (line 607)
- **Trainer profile**: business name, organization details (address, website, description), Stripe integration IDs (Epic-05), subscription status (Epic-05), platform fee percentage (Epic-05). Relationship: a User with role Trainer. (line 613)
- **Coach profile**: which trainer they work for (exactly ONE), bio and credentials, certifications, public-profile visibility setting, date joined the trainer. Relationship: belongs to one Trainer. (line 620)
- **Player profile**: player name (may differ from account name if a child), age or birth date, gender, skill level (set by trainer), school/jersey number (optional), is-child flag, link to parent account if a child, emergency contact info. Relationship: optionally a child of a parent User; associated with Trainer(s) via the Trainer-Player Association. (line 627)
- **Trainer-Player Association** (multi-trainer support): which trainer, which player, which ShareLink was used to connect them, when they connected, status (active/inactive). Relationship: join entity between Trainer and Player. (line 637)
- **ShareLink**: unique URL-safe code, type (static for players / unique for coaches), owning trainer, creator, target email (for coach links), expiration date (if applicable), maximum uses (if applicable), use count, active status. Relationship: belongs to a Trainer; referenced by registrations. (line 644)
- **Best Times / Availability**: who it belongs to (a coach or a player), day of week, start and end time, available/not-available flag, created/updated timestamps. Relationship: belongs to a Coach or a Player. (line 655)
- **Impersonation audit log**: which admin impersonated, which user was impersonated, start and end timestamps, duration, actions taken (optional detailed log). Relationship: references an admin User and a target User. (line 662)
- **Child purchase approval**: which child player, which parent must approve, which event/purchase, amount, status (pending/approved/denied/expired), request and response timestamps, expiration timestamp (48 hours), parent notes. Relationship: references a child Player, a parent User, and an event/purchase (Epic-02/Epic-05). (line 669)
- **Coach availability override**: which event, which coach, which trainer overrode, reason for override (required), timestamp. Relationship: references an Event (Epic-02), a Coach, and a Trainer. (line 679)
- **User deletion compliance record**: original user ID, original email (for legal compliance), who deleted the user, reason for deletion, when deleted, backup of original data. Relationship: references the (now anonymized) User. (line 686)

## Edge cases

| Case | Expected |
|---|---|
| Duplicate email on trainer account creation | Clear error message; email must be unique — US-01.01 (line 158) |
| Coach invite ShareLink has expired | Clear message shown, with an option to resend the invitation — US-01.08 (line 427) |
| Invited coach is already active under a different trainer | Registration blocked with an error message; a coach can only be active under one trainer at a time — US-01.08 (lines 422, 426) |
| A deactivated user attempts to log in | Login blocked with "Account deactivated. Contact support." — US-01.12 (line 517) |
| Super Admin attempts to impersonate another Super Admin | Blocked with a validation error — US-01.07 (line 400) |
| An impersonation session is left open | Auto-expires after 1 hour — US-01.07 (line 402); Business Rules "Impersonation Rules" (line 759) |
| A child's USD (or approval-required token) purchase request goes unanswered | Auto-denied after 48 hours, with a notification — US-01.05 "Implementation Notes" (line 330); Business Rules "Child Purchase Approval Workflow" (line 748) |
| Trainer assigns a coach to a time outside the coach's stated "My Times" availability | System warns the trainer; a text reason is required to override; the override is logged (event, coach, reason, overriding trainer) — US-01.10 (lines 469-471) |
| A logged-in child clicks a different trainer's ShareLink | Registration blocked; child is told to ask their parent; parent is emailed a "Review Registration" link; child is not associated with the new trainer until the parent completes it — US-01.06 "ShareLink Blocking Flow" (lines 363-369) |
| Parent removes a child from a trainer | Confirmation warns this cancels all the child's upcoming RSVPs; on confirmation the child is disassociated and their data with that trainer is soft-deleted (history preserved) — US-01.04 "Remove Child from Trainer" (lines 258-261) |
| Attempt to reactivate a deleted (not merely deactivated) user | Not possible — GDPR anonymization is permanent, unlike deactivation which is reversible — US-01.13 (line 546) |
| A new child profile has a name/age similar to an existing profile | System warns of the possible duplicate; does not block — US-01.03 "Validation" (line 229) |
| A parent with more than one trainer adds a new child | Shown a trainer-selection checklist instead of the single-trainer yes/no prompt — US-01.03 "Trainer Selection" (line 218) |
| A player with an existing account clicks a second trainer's ShareLink | Associated with the second trainer; no duplicate account created; the two trainer relationships become separate, isolated contexts — US-01.02 "Acceptance Criteria (Multi-Trainer)" (lines 184-185, 191-196) |

Boundary/failure behaviors the epic raises but does **not** state an expected
outcome for (exact rate-limit threshold/lockout behavior; whether email
verification blocks login) are listed in Open questions instead of guessed
here.

## Out of MVP scope

Verbatim from "Out of Scope (Post-MVP)" (line 90):

- Open Gym / League Instructor role
- Referee / Contractor role
- Social login (Google, Facebook, Apple)
- Two-factor authentication (2FA)
- Advanced permission customization per user
- Coaches Corner (messaging hub)
- Build-a-Bag (skill focus tool)
- Feedback tool (player feedback system)
- Custom role creation (User Role Editor full version)
- Advanced portal branding (fonts, full layout customization)
- User bulk import/export
- API access for external integrations

Verbatim note attached to that list (line 106) — reproduced as written,
including its story reference, which is incorrect (see Open questions,
analyst-raised):
> **Note**: Simple portal branding (logo upload + color selection) IS
> included in MVP - see US-01.12

## Open questions

### From the epic's "Questions / Open Issues" section (line 920, verbatim)

| ID | Question | Priority | Status | Owner |
|:---|:---|:---:|:---|:---|
| Q-01.01 | What are the specific skill level definitions? (Beginner, Intermediate, Advanced, Elite, or custom?) | P2 | Open | Client |
| Q-01.02 | How are age groups defined? (Birth year, age ranges, grade levels?) | P2 | Open | Client |
| Q-01.04 | What automated emails are required? (Welcome, password reset, invite, others?) | P1 | Open | Client |
| Q-01.05 | Email verification: Required before login or optional? | P1 | Open | Client |
| Q-01.06 | Coach availability override: Should coach be notified when overridden? | P2 | Open | Client |
| Q-01.07 | Session timeout: How long should users stay logged in? (1 day, 7 days, 30 days?) | P2 | Open | Client |

*Reference*: `05_Discussions/Open_Questions.md` for full question log (line 931)

### Additional client question embedded in US-01.06 (line 376, verbatim)

This question is **not** part of the table above — it is embedded directly
in the US-01.06 story and reuses the same ID, "Q-01.05", for a different
question than the table's Q-01.05 (see analyst-raised note below):

> **Open Question** (Q-01.05):
> - Should ALL players under 18 require parent accounts?
> - Or allow 16-18 year olds to have independent accounts?
> - COPPA compliance considerations

### Analyst-raised

- **(analyst-raised)** ID collision: "Q-01.05" is used for two different
  client questions — the table's Q-01.05 ("Email verification: Required
  before login or optional?", line 927) and the question embedded in
  US-01.06 about whether all under-18 players require parent accounts /
  COPPA (lines 376-379). These need distinct IDs before either can be
  referenced unambiguously.
- **(analyst-raised)** Direct contradiction: "Business Rules & Logic" states
  as a settled rule that "ALL players under 18 require parent-managed
  accounts (no independent accounts for minors)" (line 739, restated as
  BR-01-17), while the epic simultaneously carries the unresolved question
  above asking exactly this (US-01.06, lines 376-379). It is unclear which
  one currently governs.
- **(analyst-raised)** The open-questions table's own ID sequence skips
  Q-01.03 — it jumps from Q-01.02 to Q-01.04 (lines 924-926) — and no
  Q-01.03 appears anywhere else in the epic. Unclear whether a question was
  deleted, renumbered, or never filled in.
- **(analyst-raised, confirmed defect)** The Out-of-Scope note "Simple
  portal branding ... IS included in MVP - see US-01.12" (line 106) points
  to the wrong story. Portal branding is US-01.14 "Trainer Customizes Portal
  Branding" (line 551); US-01.12 is "Super Admin Deactivates User" (line
  507). The epic's own closing summary repeats the same mix-up: "**User
  Stories**: 12 (includes portal branding - US-01.12)" (line 1097) — and
  that count is itself off, since the epic contains 14 numbered stories
  (US-01.01–US-01.14), not 12.
- **(analyst-raised, methodology note)** Sections 10, 11, and 12 are each
  used twice in this file: §10 "User Flows" (line 805) and §10 "Acceptance
  Criteria (Epic-Level)" (line 935); §11 "Performance & Scale Targets" (line
  908) and §11 "Mockups / Design References" (line 998); §12 "Questions /
  Open Issues" (line 920) and §12 "Testing Considerations" (line 1016).
  Every back-reference in this spec cites heading text plus line number for
  this reason. Worth a fix request to the client if this source file is ever
  revised.
- **(analyst-raised)** No user story spells out the actual password-reset or
  email-verification flow. Both are asserted only as epic-level completion
  criteria ("Password reset flow works end-to-end", line 941; "Email
  verification sends and processes correctly", line 942) plus token-expiry
  business rules (1 hour / 24 hours — BR-01-4, BR-01-5). Unlike comparable
  features (e.g. US-01.01 for trainer creation, US-01.08 for coach
  invitation), there is no story walking through the request → email →
  confirm sequence for either.
- **(analyst-raised)** A few epic-level completion criteria describe
  capabilities with no dedicated story detailing the flow: "Super Admin can
  edit any user account" (AC-01-71, line 949), "Users tool shows all users
  with tool-specific search and filters" (AC-01-72, line 954), and "Trainer
  can generate ShareLinks" for the player side specifically (AC-01-73, line
  973 — the coach side is covered by US-01.08). Confirm whether these are
  intentionally left at a high level for MVP or need their own stories.
- **(analyst-raised)** Minor inconsistency between the two places performance
  targets are stated: the narrative "Performance & Scale Targets" section
  (line 908) includes "ShareLink registration: <2 seconds", which does not
  appear in the "Acceptance Criteria (Epic-Level)" § Performance checklist
  (line 985); that checklist's "Platform handles 1,000 concurrent users"
  (line 989) does not appear in the narrative section either. Worth
  reconciling into one list.
