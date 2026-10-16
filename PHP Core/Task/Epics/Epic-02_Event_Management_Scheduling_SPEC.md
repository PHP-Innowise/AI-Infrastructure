# Epic-02: Event Management & Scheduling

---

## 1. Description

**Epic-02** implements the core event creation, scheduling, and attendance tracking system for the platform. This epic enables trainers to create and manage training events, players/parents to discover and register for events, and coaches to view assignments and track attendance.

The event system supports **multiple event types** (training sessions, private sessions, small groups), **flexible pricing** (free, paid, tokens), and **smart features** like availability matching, private event invitations, and easy event duplication.

**Business Value**: Events are the core of the platform's value proposition. Without event management:
- Trainers cannot schedule and monetize training events
- Players cannot discover and register for training
- Coaches cannot see their assignments
- Platform cannot generate revenue (no paid events)
- No attendance tracking for analytics and accountability

This epic enables the complete event lifecycle from creation to completion.

---

## 2. Business Value

**Problem Statement**: 
Sports trainers currently use fragmented tools (spreadsheets, group texts, manual payment tracking) to manage training events. Players struggle to discover available events, trainers lose revenue from poor attendance, and coaches lack clear assignment visibility.

**Success Metrics**:
- ✅ 90%+ of trainers create their first event within 15 minutes of onboarding
- ✅ <3 clicks from player seeing event to RSVP confirmation
- ✅ 80%+ of players RSVP to events matching their availability
- ✅ Coach assignment confirmation rate >90%
- ✅ Attendance tracking completion rate >95%
- ✅ Average event fill rate >70% of capacity
- ✅ Private event feature usage by 50%+ of trainers

---

## 3. In Scope (MVP)

### Event Creation & Management (Trainer)
- [ ] Create events with all required attributes (title, date/time, location, coach, capacity)
- [ ] Event types: Training Session, Private Session, Small Group
- [ ] Eligibility restrictions (age, skill level, gender)
- [ ] **Dual Pricing Options**
  - Event can have BOTH USD price AND Token price
  - Players choose payment method (pay with money OR tokens)
  - Trainers can toggle either option per event
  - **Default**: Token pricing ENABLED (1 token), USD pricing DISABLED ($0)
- [ ] **Flexible Token Pricing**
  - Events can cost multiple tokens (not always 1:1)
  - Example: Premium 3-hour session = 2 tokens
  - Trainer-configurable per event
- [ ] **Recurring Events - Bulk Auto-Creation**
  - Trainer creates recurring pattern (e.g., "Every Tuesday for 3 months")
  - Platform generates individual event instances (e.g., 12 separate Tuesday events)
  - Each event is independent (can have different coaches, be edited/canceled separately)
  - Subscription holders get automatic access to each qualifying event
- [ ] **Private/Invite-Only events** (only specific players see it)
- [ ] Duplicate event functionality (copy event with new date/time)
- [ ] Edit existing events
- [ ] Cancel events with player notification
- [ ] View RSVP list
- [ ] **Coach assignment with per-event approval workflow**
  - Coach receives assignment notification
  - Coach can confirm or decline assignment
  - Event shows "Pending Coach Confirmation" until accepted
- [ ] **Player availability visibility** (see which players are available at event time using Epic-01 availability data)

### Player/Parent Experience
- [ ] Training Calendar (view all eligible events for current trainer context)
- [ ] **Separated views for multi-trainer players** (switch trainer contexts to see different calendars)
- [ ] Event details view
- [ ] RSVP for events (free events instant, paid events require payment)
- [ ] **Dual Payment Options**
  - If event has both USD and Token pricing: Player chooses payment method
  - Clear UI showing "Pay $25 OR 2 tokens"
  - Payment selection at RSVP time
- [ ] **Subscription RSVP Restrictions**
  - Players with unlimited subscription: Can RSVP to **1 event per day in advance**
  - Same-day bookings: Unlimited RSVPs (if spots available)
  - Prevents subscription holders from monopolizing all future slots
  - Clear error message: "Subscription holders can book 1 event per day in advance"
- [ ] Cancel RSVP (with 24-hour refund policy enforcement)
- [ ] **Event full indicator** (show "Event Full" when capacity reached, no RSVP button)
- [ ] My Reservations list (upcoming events)
- [ ] Private event access (only see invited events)
- [ ] Search events (tool-specific, within Training Calendar)

### Coach Experience
- [ ] My Activities dashboard (view assigned events)
- [ ] Confirm or decline event assignments
- [ ] View player roster per event
- [ ] Attendance tracking (Present, Absent, Late, Excused)
- [ ] Same-day attendance editing
- [ ] Event notes (coach preparation notes)

### Super Admin Tools
- [ ] Event Master Tool (view all events across trainers)
- [ ] Tool-specific search and filters
- [ ] Override scheduling conflicts without warnings
- [ ] Event analytics and reporting

---

## 4. Out of Scope (Post-MVP)

**Confirmed OUT of MVP**:
- ❌ **Open Gym / League events**
- ❌ **League games and schedules**
- ❌ **Referee assignments**

**Moved to Epic-08**:
- ✅ **Camps & Evaluations** - Now in Epic-08 (Forms & Registration)
  - Camps are FORMS, not calendar events
  - See Epic-08 for camp registration system

**Phase 2 Deferrals**:
- ❌ **Event Waitlist** - Simplified for MVP: Show "Event Full" only, no waitlist queue
- ❌ **Per-trainer refund policy configuration** - Using global 24-hour refund policy instead
- ❌ **True recurring event series** - Using bulk auto-creation instead (creates individual events, not linked series)
- ❌ Bulk operations (bulk edit, bulk cancel, bulk clone)
- ❌ Event templates (saved event configurations)
- ❌ Advanced scheduling suggestions (AI-powered)
- ❌ Automatic reminders (email/SMS 24hr before)
- ❌ Player feedback/ratings after events
- ❌ Video conferencing integration (virtual events)
- ❌ Equipment tracking (equipment required/provided)

---

## 5. Dependencies

### Required Before This Epic
- **Epic-01** (User Management) - Requires trainers, coaches, players to exist

### Blocks These Epics
- **Epic-03** (CRM) - CRM uses event attendance data
- **Epic-04** (LPPP) - Can assign content to events
- **Epic-05** (Payments) - Paid events require payment processing
- **Epic-06** (Marketing) - ShareLinks can link to specific events

### External Dependencies
- **Payment System**: Process payments for paid events (Epic-05, Stripe)
- **Email Service**: Notifications (RSVP confirmation, cancellation, reminders)
- **Calendar System**: Display events in calendar view
- **File Storage**: Event images/photos (optional)

---

## 6. User Roles Involved

| Role | Interaction | Permissions |
|:---|:---|:---|
| **Trainer / Business Owner** | Creates events, assigns coaches, views RSVPs, manages capacity | Full control over own events, cannot access other trainers' events |
| **Coach / Contractor** | Views assigned events, confirms/declines, tracks attendance | View-only on event details, can manage attendance for assigned events |
| **Player / Parent** | Views calendar, RSVPs to events, cancels RSVPs | Can RSVP to eligible events only, cannot create or edit events |
| **Super Admin** | Views all events, overrides conflicts, analyzes system-wide data | Full access to all events across all trainers, can override any rule |

---

## 7. User Stories & Acceptance Criteria

### US-02.01: Trainer Creates Training Event

**As a** Trainer  
**I want to** create a training event  
**So that** players can discover and register for my training

**Acceptance Criteria**:
- [ ] From Event Builder, click "Create Event"
- [ ] Fill required fields:
  - Title (e.g., "Basketball Skills Training")
  - Event Type (dropdown: Training Session, Private Session, Small Group)
  - Date and Time (start and end, date/time picker)
  - Location (dropdown from my locations)
  - Capacity (max players, number input)
- [ ] Optional fields:
  - Description (text area, supports formatting)
  - Eligibility (age range, skill level, gender)
  - Pricing (Free, Paid Money, Paid Tokens)
  - Coach assignment (select from my coaches)
- [ ] Save → Event created
- [ ] Event appears in my Event Builder list
- [ ] Event appears in eligible players' Training Calendar
- [ ] Confirmation message: "Event created successfully"

**Validation**:
- [ ] Title required (max 100 characters)
- [ ] Date/time cannot be in the past
- [ ] End time must be after start time
- [ ] Capacity must be > 0
- [ ] If paid: Price/token amount required

---

### US-02.02: Trainer Creates Private Event

**As a** Trainer  
**I want to** create a private event visible only to specific players  
**So that** I can offer exclusive sessions to select groups

**Acceptance Criteria**:
- [ ] Create event (same as US-02.01)
- [ ] Set Visibility: "Private / Invite Only"
- [ ] Select invited players:
  - **Individual player selection only** (multi-select list from trainer's players)
  - No group-based selection in MVP (Phase 2 feature)
- [ ] Save → Private event created
- [ ] Event appears in invited players' calendars ONLY
- [ ] Non-invited players do NOT see event (even if eligible by age/skill)
- [ ] Non-invited players cannot access via direct link (show "Access Denied")

**Use Cases**:
- Elite player sessions
- Scholarship athletes
- Trial sessions for prospective players
- VIP experiences

---

### US-02.03: Trainer Assigns Coach with Availability Check

**As a** Trainer  
**I want to** assign a coach to an event  
**So that** the event has an instructor

**Acceptance Criteria**:
- [ ] When creating/editing event, select Coach from dropdown
  - **Dropdown includes trainer themselves** (trainers can assign themselves as coach)
  - Also includes all coaches added by trainer (from Epic-01)
- [ ] If selected time conflicts with coach's My Times availability:
  - Show warning: "Coach [Name] is not available at this time. Continue anyway?"
  - Trainer can override with required reason (text field)
  - Override logged (who, when, event, coach, reason)
- [ ] If no conflict: Coach assigned directly
- [ ] Coach sees event in "Events to Confirm" (My Activities)
- [ ] Coach receives notification (email + in-app)

**Coach Conflict Scenarios**:
- Coach My Times: Monday 4-6pm
- Trainer assigns to Monday 7-9pm event → Warning shown
- Trainer assigns to Monday 5-7pm event → No warning (within availability)

---

### US-02.04: Trainer Sees Player Availability When Scheduling

**As a** Trainer  
**I want to** see which players are available at my event time  
**So that** I can schedule at times convenient for most players

**Acceptance Criteria**:
- [ ] When selecting event date/time in Event Builder, see availability count:
  - "15 of 20 eligible players available at this time"
  - Uses player availability data from Epic-01 (player's stated available times)
- [ ] In RSVP list, each player shows availability indicator:
  - ✅ "Available" (green): Player marked this time as available in their profile
  - ⚠️ "Unknown" (gray): Player hasn't set availability for this time
  - ❌ "Busy" (red): Player marked conflicting time
- [ ] Helps trainer optimize scheduling for better attendance

**Implementation Note**: Uses existing player availability data from Epic-01 (US-01.09), no separate availability system needed.

---

### US-02.05: Trainer Duplicates Event

**As a** Trainer  
**I want to** copy an existing event with a new date  
**So that** I can quickly create similar events without starting from scratch

**Acceptance Criteria**:
- [ ] From Event Builder list, click "Duplicate" on event row
- [ ] Modal opens with all event details pre-filled from original:
  - Title (append "- Copy" or keep same)
  - Type, location, coach, capacity, pricing, eligibility
- [ ] Change date/time (required)
- [ ] Optionally modify any other field
- [ ] Save → New event created
- [ ] Original event unchanged
- [ ] RSVPs NOT copied (new event starts empty)
- [ ] New event appears in calendar for eligible players

**Use Cases**:
- Same event at different times
- Template-like events
- Quick setup for similar events

**⚠️ Note**: For automatic recurring event creation, see "Recurring Events - Bulk Auto-Creation" feature (In Scope, line 58)

---

### US-02.06: Player Views Training Calendar

**As a** Player or Parent  
**I want to** view available training events  
**So that** I can find events to attend

**Acceptance Criteria**:
- [ ] Navigate to "Training Calendar"
- [ ] Calendar view (month, week, day toggles)
- [ ] Events displayed:
  - All public events for which player is eligible (age, skill, gender)
  - Private events where player is invited
  - NOT shown: Events where player ineligible, non-invited private events
- [ ] Each event shows: Title, date/time, location, capacity (X/Y), price
- [ ] Click event → Opens event details modal
- [ ] Search events (tool-specific): Search by title, location, date
- [ ] Filter events: By date range, by type, by location
- [ ] Availability indicator (if player set availability preferences):
  - ✅ Badge: "Matches your availability"
  - ❌ Badge: "Conflicts with your availability"

**✅ Updated**: D-SCOPE-005 (Tool-specific search, not global)

---

### US-02.07: Player RSVPs to Event

**As a** Player or Parent  
**I want to** RSVP to a training event  
**So that** I can attend and secure my spot

**Acceptance Criteria (Free Event)**:
- [ ] View event details
- [ ] Click "RSVP" button
- [ ] If child account: Requires parent approval (US-01.04)
- [ ] If space available: Instant confirmation
  - Status: "Registered"
  - Event added to "My Reservations"
  - Confirmation message: "You're registered!"
  - Confirmation email sent
- [ ] If event full: Show "Event Full - No spots available" message
  - RSVP button disabled
  - Suggest: Check back later or contact trainer

**Acceptance Criteria (Paid Event - Money or Tokens)**:
- [ ] View event details, see price ($X or Y tokens)
- [ ] Click "Register & Pay"
- [ ] If child account: Requires parent approval (see US-01.04)
- [ ] Redirect to payment (Epic-05):
  - Enter payment method OR use tokens
  - Process payment
- [ ] After payment success:
  - Status: "Registered"
  - Event added to "My Reservations"
  - Confirmation message + email
  - Receipt provided
- [ ] If event full: Show "Event Full" message, registration disabled

**Validation**:
- [ ] Cannot RSVP twice to same event (show "Already registered")
- [ ] Cannot RSVP if ineligible (age/skill/gender)
- [ ] Cannot RSVP to past events

---

### US-02.08: Player Cancels RSVP

**As a** Player or Parent  
**I want to** cancel my event registration  
**So that** I can free up my spot if I can't attend

**Acceptance Criteria (Free Event)**:
- [ ] From "My Reservations", click event
- [ ] Click "Cancel RSVP"
- [ ] Confirmation: "Are you sure? This will free your spot."
- [ ] Confirm → RSVP canceled
  - Removed from event roster
  - Spot opens for other players to register
  - Cancellation email sent
- [ ] Cannot cancel after event start time (or X hours before - policy TBD)

**Acceptance Criteria (Paid Event)**:
- [ ] Same as free event, plus refund processing:
- [ ] **24-Hour Refund Policy**:
  - If **≥24 hours before event**: **Full refund** (money or tokens returned)
  - If **<24 hours before**: **No refund** (payment/tokens forfeited)
  - If **trainer cancels**: **Always full refund** (regardless of timing)
  - Trainer can manually override via Stripe if needed
- [ ] Refund processed automatically (Epic-05)
- [ ] Refund confirmation email sent

**Special Case**: If child initiated cancellation, parent approval required (same as purchase approval)

---

### US-02.10: Coach Confirms Event Assignment

**As a** Coach  
**I want to** confirm or decline event assignments  
**So that** trainers know my availability

**Acceptance Criteria**:
- [ ] Navigate to "My Activities"
- [ ] "Events to Confirm" section shows pending assignments
- [ ] Each pending event shows: Title, date/time, location, # players RSVP'd
- [ ] Actions per event:

**Option A - Confirm**:
- [ ] Click "Confirm"
- [ ] Session moves to "Assigned Sessions" list
- [ ] Status updated to "Confirmed"
- [ ] Trainer notified (in-app + email)
- [ ] Shows in coach's calendar

**Option B - Decline**:
- [ ] Click "Decline"
- [ ] Optional: Reason for declining (text field)
- [ ] Session removed from coach's view
- [ ] Trainer notified (in-app + email): "[Coach] declined [Event]. Find replacement."
- [ ] Event still exists, just needs new coach assignment

**Auto-confirm** (optional): If coach doesn't respond within 48 hours, auto-confirm (trainer preference setting)

---

### US-02.11: Coach Tracks Attendance

**As a** Coach  
**I want to** mark player attendance for my events  
**So that** attendance records are accurate

**Acceptance Criteria**:
- [ ] After event start time, open event from "My Activities"
- [ ] "Take Attendance" button available
- [ ] Player roster list with attendance options per player:
  - **Present** (default if player RSVP'd)
  - **Absent** (no-show)
  - **Late** (arrived late)
  - **Excused** (excused absence)
- [ ] Select status for each player
- [ ] Save → Attendance recorded
- [ ] Attendance visible to:
  - Trainer (in event details and analytics)
  - Super Admin (in reports)
  - Player/Parent (in their history - optional)

**Edit Rules**:
- [ ] Coach can edit attendance **same day only**
- [ ] After midnight: Attendance locked for coach
- [ ] Trainer can edit anytime (override coach's entry)
- [ ] Super Admin can edit anytime

**Validation**:
- [ ] Cannot mark attendance for events not started yet
- [ ] Cannot mark attendance for events not assigned to coach
- [ ] If player canceled RSVP: Don't show in attendance list

---

### US-02.12: Trainer Views RSVP List

**As a** Trainer  
**I want to** view who's registered for my event  
**So that** I can prepare for the event

**Acceptance Criteria**:
- [ ] From Event Builder, click event
- [ ] "RSVP List" tab shows:
  - Player name, age, skill level
  - RSVP timestamp (when they registered)
  - Payment status (for paid events: Paid, Pending, Refunded)
  - Availability match indicator (green/yellow/red based on player availability from Epic-01)
- [ ] Count shown: "15 of 20 registered" (with capacity indicator)
- [ ] If full: Show "Event Full - No spots available"
- [ ] Actions:
  - Manually add player (if space available)
  - Remove player (with refund if paid event)
  - Export list (CSV for record-keeping)

---

### US-02.13: Trainer Cancels Event

**As a** Trainer  
**I want to** cancel an event  
**So that** players know the event won't happen

**Acceptance Criteria**:
- [ ] From Event Builder, select event
- [ ] Click "Cancel Event"
- [ ] Confirmation: "This will notify all registered players and process refunds. Continue?"
- [ ] Required: Cancellation reason (text field)
- [ ] Confirm → Event canceled
  - Status: "Canceled"
  - All registered players notified (email + in-app)
  - Refunds processed automatically for paid events (Epic-05, 24-hour policy D-BUS-005)
  - Event removed from Training Calendar (or shown as "Canceled")
  - Coach notified (assignment canceled)

**Edge Cases**:
- [ ] Cannot cancel event less than X hours before start (policy TBD, e.g., 24 hours)
- [ ] Super Admin can cancel anytime (override policy)
- [ ] If event already started or completed: Cannot cancel (mark as "Completed" instead)

---

### US-02.14: Trainer Edits Event

**As a** Trainer  
**I want to** modify event details after creation  
**So that** I can correct mistakes or adapt to changes

**Acceptance Criteria**:
- [ ] From Event Builder, select event, click "Edit"
- [ ] All event fields editable (except past events)
- [ ] Editable fields: Title, description, date/time, location, capacity, coach, pricing, eligibility
- [ ] Changes that trigger notifications:
  - Date/time changed → Notify all RSVP'd players
  - Location changed → Notify all RSVP'd players
  - Coach changed → Notify old and new coach
  - Price changed → Notify RSVP'd players (if price increased, allow cancellation with refund)
- [ ] Save → Changes applied
- [ ] Updated event details reflected in all views (calendar, My Reservations, My Activities)

**Capacity Changes**:
- [ ] Increase capacity: Allow more RSVPs (spots become available for new registrations)
- [ ] Decrease capacity: If new capacity < current RSVPs:
  - Warning: "This will exceed capacity. Remove players or increase capacity."
  - Cannot save until resolved

**Validation**:
- [ ] Cannot edit events in the past
- [ ] Cannot edit canceled events (create new event instead)
- [ ] Date/time validation (end after start, not in past)

---

### US-02.15: Super Admin Views All Events

**As a** Super Admin  
**I want to** view all events across all trainers  
**So that** I can monitor platform activity and support trainers

**Acceptance Criteria**:
- [ ] Navigate to "Event Master Tool"
- [ ] List all events from all trainers
- [ ] Tool-specific search: Search by title, trainer name, location, date
- [ ] Filters:
  - Trainer (dropdown/autocomplete)
  - Location (dropdown)
  - Date range (date picker)
  - Event type (Training, Private, Small Group)
  - Status (Active, Canceled, Completed)
- [ ] Pagination (show 50 events per page)
- [ ] Click event → View full details
- [ ] Actions:
  - Edit any event (same as trainer)
  - Cancel any event (override trainer restrictions)
  - Override scheduling conflicts (no warnings needed)
  - View RSVP list
  - Export events (CSV for reporting)

**✅ Updated**: D-SCOPE-005 (Tool-specific search), SAQ9 (Super Admin overrides without warnings)

---

## 8. Data Requirements

### What Information Needs to Be Stored

**For Events**:
- Event unique identifier
- Title and description
- Event type (Training Session, Private Session, Small Group)
- Trainer who created it
- Date and time (start and end)
- Location
- Assigned coach(es)
- Capacity (maximum RSVPs)
- Current RSVP count
- Visibility (Public, Private/Invite-Only)
- If private: List of invited players/groups
- Pricing type (Free, Paid Money, Paid Tokens)
- If paid: Amount
- Eligibility restrictions (age range, skill level, gender)
- Status (Active, Canceled, Completed)
- Created and updated timestamps
- If canceled: Reason, who canceled, when

**For RSVPs**:
- Which player
- Which event
- RSVP timestamp
- Status (Registered, Canceled, Attended, No-Show)
- If paid: Payment reference, refund status
- If canceled: Cancellation timestamp, reason

**For Attendance**:
- Which event
- Which player
- Attendance status (Present, Absent, Late, Excused)
- Recorded by (coach)
- Recorded timestamp
- If edited: Edit history (who, when, old value, new value)

**For Coach Assignments**:
- Which event
- Which coach
- Assignment status (Pending, Confirmed, Declined)
- If declined: Reason
- Confirmation timestamp
- If availability conflict: Override flag, override reason, who overrode

**For Private Event Invitations**:
- Which event
- Which player or group invited
- Invitation timestamp

**For Event Duplication**:
- Original event reference (for tracking/analytics)
- Duplication timestamp
- Who duplicated

---

## 9. Business Rules & Logic

### Event Creation Rules

**Required Fields**:
- Title (max 100 characters)
- Event type
- Date and time (start and end)
- Location
- Capacity (minimum 1)

**Date/Time Validation**:
- Start time cannot be in the past
- End time must be after start time
- Duration should be reasonable (warn if >8 hours, max 24 hours)

**Pricing Rules**:
- If "Paid", amount must be > 0
- Free events: Amount = 0
- Token events: Token amount must be > 0

**Capacity Rules**:
- Must be positive integer
- Minimum 1, maximum 999 (reasonable limit)
- If changing capacity: Cannot reduce below current RSVP count

### Event Visibility Rules

**Public Events**:
- Shown to all eligible players on Training Calendar
- Eligibility determined by: Age, skill level, gender restrictions
- If no restrictions: Shown to all players associated with trainer

**Private Events**:
- Shown ONLY to invited players
- Even if eligible by age/skill/gender, non-invited players don't see it
- Cannot be discovered via search by non-invited players
- Direct link access: "Access Denied" for non-invited players

### RSVP Rules

**General**:
- Player can only RSVP once per event
- Cannot RSVP to past events
- Cannot RSVP if capacity reached (show "Event Full" message)
- Must meet eligibility criteria (age, skill, gender)

**Free Events**:
- Instant RSVP (no payment step)
- Confirmation immediate

**Paid Events**:
- Requires payment before confirmation
- RSVP not confirmed until payment successful
- If payment fails: RSVP not registered, spot still available

**Child Accounts**:
- All RSVPs (free or paid) require parent approval
- Status: "Pending Parent Approval" until parent confirms
- 48-hour expiry on approval requests

### Cancellation & Refund Rules

**24-Hour Refund Policy**:

**Player Cancels RSVP**:
- Cannot cancel after event starts
- **≥24 hours before event**: Full refund (money or tokens returned)
- **<24 hours before**: No refund (payment/tokens forfeited)
- Trainer can manually override via Stripe to issue refund if needed

**Trainer Cancels Event**:
- **Always full refund** to all players (regardless of timing)
- All players notified
- Coach assignments canceled
- Event marked "Canceled" (not deleted, for history)

### Coach Assignment Rules

**Assignment Process**:
- Trainer assigns coach to event
  - **Trainers can assign themselves as coach** (trainer appears in coach dropdown)
  - Can also assign any coach added via Epic-01 (Coach Management)
- If coach availability conflict (outside My Times): Warning shown, can override with reason
- Coach receives notification
- Status: "Pending" until coach confirms

**Coach Confirmation**:
- Coach can confirm or decline
- If decline: Trainer notified to find replacement
- Auto-confirm after 48 hours (optional trainer setting)

**Coach Limitations**:
- Coach cannot be assigned to overlapping events
- System warns on double-booking
- Trainer can override with reason (logged)

### Attendance Tracking Rules

**Recording Attendance**:
- Can only mark attendance after event start time
- Coach can mark for assigned events only
- Default status: "Present" for all RSVP'd players

**Attendance Statuses**:
- **Present**: Player attended
- **Absent**: No-show (player didn't attend)
- **Late**: Player arrived late but participated
- **Excused**: Excused absence (coach discretion)

**Edit Permissions**:
- Coach: Can edit same day only (until midnight)
- Trainer: Can edit anytime
- Super Admin: Can edit anytime
- After midnight: Coach view changes to read-only

### Event Duplication Rules

**Duplication Process**:
- Copies all event details (title, type, location, coach, capacity, pricing, eligibility)
- Does NOT copy: RSVPs, attendance records
- New event has new unique identifier
- Date/time must be changed (required)
- Original event reference stored (for analytics)

### Player Availability Integration Rules

**Availability Matching**:
- Player's availability (from Epic-01) compared to event date/time
- Match indicator shown to trainer:
  - ✅ Green: Event fully within player's stated available times
  - ⚠️ Gray: Player hasn't set availability for this time
  - ❌ Red: Event conflicts with player's "not available" times
- Indicators shown to trainer only (players just see event)

---

## 10. User Flows

### Flow 1: Trainer Creates and Publishes Event

1. Trainer logs in to Trainer dashboard
2. Navigate to "Event Builder"
3. Click "+ Create Event"
4. Fill required fields:
   - Title: "Basketball Skills - Shooting"
   - Type: Training Session
   - Date: Next Monday, Time: 6:00 PM - 8:00 PM
   - Location: Select "Main Gym"
   - Capacity: 20
5. Optional: Set eligibility (Age: 10-14, Skill: Intermediate)
6. Set pricing: $25 per player
7. Assign coach: Select "Coach Mike"
8. If coach conflict: System shows warning, trainer adds reason, continues
9. Click "Create Event"
10. System creates event, assigns status "Active"
11. Confirmation: "Event created! Visible to eligible players."
12. Event appears in Training Calendar for eligible players
13. Coach Mike receives notification: "You're assigned to [Event]. Confirm?"

### Flow 2: Player Discovers and RSVPs to Paid Event

1. Player/Parent logs in
2. Navigate to "Training Calendar"
3. Browse calendar (month view)
4. See event: "Basketball Skills - Shooting" on Monday 6 PM
5. Notice green badge: "Matches your availability" (based on availability preferences)
6. Click event → Event details modal opens
7. Review: Title, description, date/time, location, price ($25), spots (5/20)
8. Click "Register & Pay"
9. Redirect to payment (Epic-05):
   - Enter payment method or select "Pay with Tokens"
   - Confirm payment
10. Payment successful
11. RSVP confirmed: Status "Registered"
12. Confirmation message: "You're registered! See you Monday at 6 PM."
13. Confirmation email sent with event details
14. Event added to "My Reservations"

### Flow 3: Event Full - Player Cannot Register

1. Player views event: "Basketball Skills - Shooting"
2. Sees capacity: 20/20 (full)
3. Message: "Event Full - No spots available"
4. RSVP button disabled
5. Suggestion shown: "Check back later or contact trainer directly"
6. **Later**: If another player cancels RSVP
7. Spot opens (capacity becomes 19/20)
8. Event appears as available again in Training Calendar
9. Next player to view event can now register (first-come, first-served)

### Flow 4: Coach Confirms Event and Tracks Attendance

1. Coach logs in
2. Navigate to "My Activities"
3. "Events to Confirm" shows: "Basketball Skills - Shooting" on Monday
4. Review details: Date, time, location, 20 players RSVP'd
5. Click "Confirm"
6. Event moves to "Assigned Events"
7. Trainer notified: "Coach Mike confirmed [Event]"
8. **Day of event**: Monday 6 PM, event happens
9. After event (7:30 PM), coach opens event
10. Click "Take Attendance"
11. Player roster shows 20 players
12. Mark each:
    - 18 players: Present
    - 1 player: Late
    - 1 player: Absent
13. Save → Attendance recorded
14. Attendance visible to trainer in event details
15. Analytics updated (attendance rate: 95%)

### Flow 5: Trainer Duplicates Event for Next Week

1. Trainer in Event Builder
2. Views past event: "Basketball Skills - Shooting" (Monday 6 PM)
3. Clicks "Duplicate" button
4. Modal opens with all details pre-filled
5. Changes date: Next Monday (same time: 6-8 PM)
6. Optionally changes coach: Select "Coach Sarah" instead
7. Click "Create Duplicate"
8. New event created with new ID
9. Original event unchanged (RSVPs preserved)
10. New event has 0 RSVPs (starts fresh)
11. New event appears in Training Calendar for eligible players

### Flow 6: Trainer Creates Private Event for Elite Players

1. Trainer creates event (same as Flow 1)
2. Set Visibility: "Private / Invite Only"
3. Click "Select Invitees"
4. Modal opens with player list (individual players only, no groups in MVP)
5. Select specific players:
   - Check: "John Smith"
   - Check: "Sarah Johnson"
   - Check: "Mike Williams"
   - Check: "Emily Davis"
   - Check: "Alex Brown"
6. Total: 5 players invited
7. Save invitations
8. Finish creating event
9. Event created, visible ONLY to 5 invited players
10. Invited players see event in their Training Calendar
11. Non-invited players do NOT see event (even if age/skill match)
12. If non-invited player gets direct link: "Access Denied" message

### Flow 7: Player Cancels Paid Event with Refund

1. Player views "My Reservations"
2. Sees upcoming event: "Basketball Skills - Shooting" (paid $25)
3. Event is in 3 days (>24 hours away)
4. Clicks "Cancel RSVP"
5. Confirmation: "You'll receive a full refund ($25). Continue?"
6. Confirm cancellation
7. RSVP canceled, spot freed for other players to register
8. Refund processed automatically (Epic-05)
9. Refund confirmation email: "Your $25 refund will appear in 3-5 days"
10. Event removed from "My Reservations"

**Note**: If <24 hours before event, player sees: "No refund available (within 24-hour window). Continue?"

---

## 11. Performance & Scale Targets

**Response Times**:
- Event creation: <2 seconds
- Training Calendar load with 100 events: <3 seconds
- RSVP confirmation: <1 second (free events), <3 seconds (paid events including payment)
- Event search: <1 second for query results

**Scale**:
- Support 10,000+ events per trainer (historical)
- Display 100+ events in calendar view (paginated/lazy-loaded)
- Handle 50 concurrent RSVPs to same event (capacity management)
- Support 1,000+ players RSVPing across multiple events simultaneously

**Note**: Specific technical implementation (caching, indexing, etc.) decided by developer.

---

## 12. Questions / Open Issues

| ID | Question | Priority | Status | Owner |
|:---|:---|:---:|:---|:---|
| Q-02.02 | Auto-confirm coach assignments after 48 hours, or require explicit confirmation always? | P2 | Open | Client |
| Q-02.08 | Event reminders: Email/SMS 24hrs before? (Defer to Post-MVP?) | P2 | Open | Client |
| Q-02.09 | Can trainer manually add player to full event (override capacity)? | P2 | Open | Client |

---

## 13. Acceptance Criteria (Epic-Level)

This Epic is complete when:

**Event Creation (Trainer)**:
- [ ] Trainer can create training events with all required fields
- [ ] Trainer can create private events visible only to invited players
- [ ] Trainer can assign coaches with availability conflict warnings
- [ ] Trainer can see player Best Times availability when scheduling
- [ ] Trainer can duplicate existing events with new dates
- [ ] Trainer can edit events (with player notifications for major changes)
- [ ] Trainer can cancel events (with refunds and notifications)

**Event Discovery (Player/Parent)**:
- [ ] Player sees all eligible events in Training Calendar
- [ ] Player sees private events if invited (does not see if not invited)
- [ ] Player can search events within calendar (tool-specific)
- [ ] Player sees their events in separated views per trainer context
- [ ] Calendar view works on desktop and mobile

**RSVP & Payment**:
- [ ] Player can RSVP to free events (instant confirmation)
- [ ] Player can RSVP to paid events (payment processed, then confirmation)
- [ ] Player sees "Event Full" message when capacity reached (no RSVP allowed)
- [ ] Player can cancel RSVP (with refund if applicable)
- [ ] Child RSVPs require parent approval workflow

**Coach Experience**:
- [ ] Coach sees assigned events in My Activities
- [ ] Coach can confirm or decline assignments
- [ ] Coach can mark attendance (Present, Absent, Late, Excused)
- [ ] Coach can edit attendance same day only (locked after midnight)
- [ ] Trainer can view and edit attendance anytime

**Super Admin**:
- [ ] Super Admin can view all events across all trainers (Event Master Tool)
- [ ] Super Admin can search and filter events (tool-specific)
- [ ] Super Admin can override scheduling conflicts without warnings
- [ ] Super Admin can edit/cancel any event

**Data Integrity**:
- [ ] RSVP count never exceeds capacity (capacity management works)
- [ ] Attendance records immutable after coach edit window
- [ ] Refunds processed automatically on cancellations
- [ ] Event history preserved (canceled events not deleted)

**Performance**:
- [ ] Calendar loads in <3 seconds with 100 events
- [ ] Event creation completes in <2 seconds
- [ ] RSVP confirmation in <1 second (free), <3 seconds (paid)
- [ ] Platform handles 50 concurrent RSVPs to same event

**Approval**:
- [ ] Demo approved
- [ ] All P0 and P1 questions resolved
- [ ] Refund policy confirmed
- [ ] Private event functionality validated

---

## 14. Mockups / Design References

**Key Screens to Design**:
1. Event Builder - Event list (trainer)
2. Event Builder - Create/Edit event form
3. Event Builder - Private event invitation selector
4. Event Builder - RSVP list view
5. Training Calendar - Month/week/day views (player)
6. Training Calendar - Event details modal
7. Training Calendar - RSVP confirmation flow
8. Training Calendar - Event Full state (capacity reached)
9. My Reservations - Upcoming events list (player)
10. My Activities - Assigned events list (coach)
11. My Activities - Attendance tracking interface
12. Event Master Tool - All events list (super admin)

---

## 15. Testing Considerations

**What Should Be Tested**:

**Functional Testing**:
- All user stories and acceptance criteria met
- All user flows work end-to-end
- All business rules enforced correctly
- All validation rules working

**Key Scenarios to Test**:
- Trainer creates public event, players see it and RSVP
- Trainer creates private event, only invited players see it
- Event reaches capacity, next player sees "Event Full" message
- Player cancels RSVP, spot opens for others to register
- Player cancels paid event, refund processed per 24-hour policy
- Trainer assigns coach with conflict, warning shown, override logged
- Coach confirms event, marks attendance
- Trainer duplicates event, new event created with no RSVPs
- Trainer edits event date, all RSVP'd players notified
- Trainer cancels event, all refunds processed

**Edge Cases**:
- Concurrent RSVPs to last spot (capacity management - only one succeeds)
- Coach declines multiple assignments (trainer notification spam)
- Event capacity reduced below current RSVPs (validation prevents this)
- Player tries to access private event via direct link (access denied)
- Coach tries to edit attendance after midnight (read-only)
- Child RSVP requires parent approval, approval expires (48 hours)

**Security Testing**:
- Players cannot RSVP to ineligible events (age/skill/gender)
- Players cannot access private events they're not invited to
- Trainers cannot see/edit other trainers' events
- Coaches cannot edit events (only view and attendance)
- Non-authenticated users cannot access any events

**Performance Testing**:
- Calendar load with 100+ events
- 50 concurrent RSVPs to same event
- 1,000 events in Event Builder list (pagination)
- Event search with large dataset (10,000+ events)

**Note**: Developer chooses specific testing tools and frameworks.

---

## 16. Implementation Notes

**Suggested Implementation Order**:
1. Event creation basic fields
2. Training Calendar view and event display
3. RSVP system (free events first, then paid)
4. Capacity management
5. Private events and invitations
6. Coach assignment and My Activities
7. Attendance tracking
8. Event duplication
9. Player availability integration (use Epic-01 data)
10. Super Admin Event Master Tool

**Integration Points with Epic-05 (Payments)**:
- Paid event RSVP triggers payment flow
- Payment success triggers RSVP confirmation
- Event cancellation triggers refund processing
- Refund policy enforcement

**Integration Points with Epic-01 (User Management)**:
- Coach assignment uses My Times availability
- Player eligibility uses age, skill level from profile
- Player availability data integration for scheduling visibility
- Parent approval workflow for child RSVPs

**Security Requirements** (High-Level):
- Validate user permissions on all event operations
- Prevent RSVP capacity bypass attacks (race conditions)
- Ensure private events truly private (no leaks via search/links)
- Log all override actions (capacity, conflicts)
- Prevent injection attacks on event descriptions (sanitize input)

**Notification Requirements**:
- RSVP confirmation (email + in-app)
- Event changes (date, location, cancellation)
- Coach assignment notification
- Refund confirmation
- Attendance recorded (optional: notify players)

*Note: Specific email service and notification system chosen by developer*

---

**Priority**: P0 - Core Platform Value (blocks revenue)
**Complexity**: High (capacity management, payment integration, multi-role interactions)

**Note**: This specification focuses on **business requirements** and **user needs**. Technical implementation details (database schema, API design, caching strategies, specific technologies) are decided by the development team based on their expertise.

