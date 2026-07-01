# Epic-03: CRM & Player Management

---

## 1. Description

**Epic-03** implements the Customer Relationship Management (CRM) system for trainers to manage their player base. This epic enables trainers to organize, segment, and understand their players through labels, flags, notes, attendance history, and analytics.

The CRM is the central hub for player management, supporting **private event invitations**, **performance tracking**, and **business insights**. It includes a **Quick View dashboard** with key metrics like Top Players, attendance trends, and flag alerts.

**Business Value**: The CRM transforms the platform from a scheduling tool into a complete player management platform. Without CRM:
- Trainers cannot segment players for targeted communications
- No way to track player engagement and at-risk players
- Cannot identify top performers or scholarship candidates
- No business intelligence for growth decisions
- Limited ability to personalize training experiences

This epic enables data-driven training businesses and player retention strategies.

---

## 2. Business Value

**Problem Statement**: 
Trainers currently use spreadsheets, paper notes, and memory to track player information, leading to missed opportunities for engagement, poor retention, and inability to identify trends. They cannot easily segment players for targeted communications or identify at-risk players.

**Success Metrics**:
- ✅ 80%+ of trainers use labels/flags within first month
- ✅ 60%+ of trainers check Quick View dashboard weekly
- ✅ Average 3-5 labels created per trainer
- ✅ Segmentation used for 50%+ of private events
- ✅ Top Players list engagement (clicks) >70% of trainers
- ✅ At-risk player identification improves retention by 15%

---

## 3. In Scope (MVP)

### CRM Tools (Trainer)
- [ ] Player list view (all players in trainer's organization)
- [ ] Tool-specific search (name, email, team, school, attendance)
- [ ] Player detail view (profile, attendance history, notes, flags, labels)
- [ ] Custom labels system (create, apply, filter)
- [ ] System-defined flags (8 flag types: Behavior, High no-show, Injured, Medical restriction, Scholarship, Financial aid, Contact priority, Attendance risk)
- [ ] Notes system (general notes + per-event notes)
- [ ] Segmentation and filtering (skill, age, gender, attendance, labels, flags)
- [ ] SharedLink invitation and tracking

### Quick View Dashboard
- [ ] Key metrics summary (events this week, RSVPs)
- [ ] Top Players algorithm (most events attended in last 90 days)
- [ ] Flag alerts (count by flag type)
- [ ] Attendance trends
- [ ] ShareLink tracking (link opens/clicks)
- [ ] **Coach Hours Tracking**
  - Track total hours/sessions per coach (for trainer reference)
  - Show "Coach has done 200 hours" or "covered 50 events"
  - Analytics for trainers to manage external coach payments
  - Note: Platform does NOT handle coach payments (trainers pay externally)

### Player Profiles (Coach View)
- [ ] **Event-Scoped Player Access** (coaches see ONLY players assigned to their specific events)
  - Coach assigned to zero events → sees zero players
  - Coach assigned to Event A → sees all players RSVP'd to Event A
  - Coach can click player to view profile, but cannot browse all trainer's players
  - No full player list browsing (trainer controls access to player data)
- [ ] Basic player information and attendance
- [ ] Add per-event feedback/notes
- [ ] View flags and labels (read-only)
- [ ] ShareLink invitation (coaches can invite players)
- [ ] **Player search limited to assigned events** (cannot search across all trainer's players)

### Super Admin CRM Access
- [ ] View all players across all trainers
- [ ] Search and filter system-wide
- [ ] Apply flags across trainers
- [ ] Quick View dashboard (system-wide metrics)

---

## 4. Out of Scope (Post-MVP)

**Phase 2 Deferrals**:
- ❌ Advanced analytics and reporting (charts, drill-downs)
- ❌ Player data export (CSV, PDF, Excel)
- ❌ Saved segments (bookmarked filter combinations)
- ❌ Automated flag application (e.g., auto-flag high no-show)
- ❌ Player messaging (in-app direct messages)
- ❌ Parent/player view of coach feedback (read-only display)
- ❌ Build a Bag (player-selected skills to work on)
- ❌ Bulk operations (bulk label, bulk flag, bulk export)
- ❌ Coach feedback ratings (star ratings, structured feedback)
- ❌ Player comparison (side-by-side player metrics)
- ❌ Revenue attribution per player (lifetime value)
- ❌ Advanced Top Players algorithm (weighted by event type, recency)
- ❌ Regional breakdowns (if multi-region)

---

## 5. Dependencies

### Required Before This Epic
- **Epic-01** (User Management) - Players, coaches, trainers must exist
- **Epic-02** (Event Management) - Attendance data comes from events

### Blocks These Epics
- **Epic-04** (LPPP) - Can assign content to player segments
- **Epic-06** (Marketing) - ShareLinks tracked in CRM

### External Dependencies
- **File Storage**: For player photos/avatars (optional)
- **Analytics System**: For dashboard metrics (can use database queries)

---

## 6. User Roles Involved

| Role | Interaction | Permissions |
|:---|:---|:---|
| **Trainer / Business Owner** | Manages all players, applies labels/flags, views dashboard | Full CRM access for own organization only |
| **Coach / Contractor** | Views assigned players, adds event feedback, applies flags | Limited to players in assigned events |
| **Player / Parent** | Manages own profile (Epic-01) | Cannot access CRM tools |
| **Super Admin** | Views all players across all trainers, system-wide analytics | Full access to all CRM data, can apply flags across trainers |

---

## 7. User Stories & Acceptance Criteria

### US-03.01: Trainer Views Player List

**As a** Trainer  
**I want to** view all players in my organization  
**So that** I can manage my player base

**Acceptance Criteria**:
- [ ] Navigate to "CRM Tools" or "Players"
- [ ] List view shows all players associated with trainer's organization
- [ ] Players included from:
  - SharedLink invitations (players who joined via link)
  - Event registrations (players who RSVP'd to events)
  - Coach invitations (coaches who invited players)
- [ ] Each player row shows:
  - Name and photo/avatar
  - Age, gender
  - Skill level
  - Labels (badges)
  - Flags (icons/badges)
  - Attendance rate (% attended)
  - Last activity date
- [ ] Click player row → Opens player detail view
- [ ] Pagination (50 players per page)
- [ ] Sort by: Name (A-Z, Z-A), Last Activity, Attendance Rate

**Performance**:
- [ ] List loads in <2 seconds for 500 players

---

### US-03.02: Trainer Searches Players

**As a** Trainer  
**I want to** search my player list  
**So that** I can quickly find specific players

**Acceptance Criteria**:
- [ ] Search bar at top of player list
- [ ] Search by:
  - Player name (first, last, full name)
  - Parent name or email (for child accounts)
  - Team, school, or club (if present in profile)
- [ ] Real-time search (results update as typing)
- [ ] Search results show matching players
- [ ] Clear search button (X icon)
- [ ] No results message: "No players found. Try different search terms."

**Tool-Specific Search**:
- [ ] Search scoped to CRM tool only (not global)
- [ ] Matches partial strings (e.g., "John" finds "Johnny")

---

### US-03.03: Trainer Creates and Applies Labels

**As a** Trainer  
**I want to** create custom labels and apply them to players  
**So that** I can organize players into meaningful groups

**Acceptance Criteria - Create Label**:
- [ ] From CRM Tools, click "Manage Labels" or "+ New Label"
- [ ] Modal opens: Create Label
- [ ] Fields:
  - Label name (required, max 50 characters)
  - Color (color picker with presets)
- [ ] Save → Label created
- [ ] Label appears in label list

**Acceptance Criteria - Apply Label**:
- [ ] From player detail view, click "+ Add Label"
- [ ] Dropdown shows all created labels
- [ ] Select label(s) to apply (multi-select)
- [ ] Save → Labels applied to player
- [ ] Labels show as colored badges in player detail and list view
- [ ] Player can have multiple labels

**Acceptance Criteria - Manage Labels**:
- [ ] Edit label: Change name or color
- [ ] Delete label: Remove label from all players (confirmation required)
- [ ] Label usage count shown: "Applied to 12 players"

**Use Cases**:
- "Elite Squad" (top performers)
- "Scholarship Candidates"
- "Needs Improvement"
- "Summer Camp 2026"
- "Beginners", "Advanced"

---

### US-03.04: Trainer Applies System Flags

**As a** Trainer  
**I want to** apply system-defined flags to players  
**So that** I can track important player statuses

**Acceptance Criteria - Apply Flag**:
- [ ] From player detail view, click "+ Add Flag"
- [ ] Dropdown shows 8 system-defined flags:
  1. **Behavior** (behavioral issues)
  2. **High no-show rate** (attendance concern)
  3. **Injured** (medical - cannot participate)
  4. **Medical restriction** (can participate with restrictions)
  5. **Scholarship** (financial aid recipient)
  6. **Financial aid** (payment assistance)
  7. **Contact priority** (requires special attention)
  8. **Attendance risk** (at risk of dropping out)
- [ ] Select flag from list
- [ ] Optional: Add note explaining flag (text area)
- [ ] Save → Flag applied
- [ ] Flag appears as badge/icon in player detail and list
- [ ] Timestamp and who applied recorded

**Acceptance Criteria - View Flags**:
- [ ] Player list shows flag icons (small badges)
- [ ] Player detail shows full flag list with notes
- [ ] Flag count shown per player: "3 flags"

**Acceptance Criteria - Remove Flag**:
- [ ] Click flag → "Remove Flag" option
- [ ] Confirmation: "Mark as resolved?"
- [ ] Flag removed from active flags
- [ ] Flag history preserved (audit log)

**Permissions**:
- [ ] Trainer: Can apply, view, remove all flags for own players
- [ ] Coach: Can apply, view flags for assigned players (optional MVP)
- [ ] Super Admin: Can apply, view, remove flags across all trainers

---

### US-03.05: Trainer Adds Notes to Player

**As a** Trainer  
**I want to** add notes to player profiles  
**So that** I can track important information and context

**Acceptance Criteria - General Notes**:
- [ ] From player detail view, "Notes" section
- [ ] Click "+ Add Note"
- [ ] Text area (up to 1000 characters)
- [ ] Save → Note added
- [ ] Note appears in chronological list (most recent first)
- [ ] Note shows: Text, who added, timestamp

**Acceptance Criteria - Per-Event Notes**:
- [ ] From player detail view, "Event History" section
- [ ] Each event row has "+ Add Note" button
- [ ] Text area opens (tied to specific event)
- [ ] Save → Note added to event
- [ ] Note visible in player history: "[Event Title] - [Date]: [Note]"

**Acceptance Criteria - Edit/Delete Notes**:
- [ ] Trainer can edit own notes within 24 hours
- [ ] Trainer can delete own notes anytime
- [ ] Cannot edit/delete coach notes (read-only for trainer)
- [ ] Super Admin can edit/delete any notes

**Use Cases**:
- "Parent prefers email communication"
- "Bring own basketball"
- "Working on left-hand dribbling"
- "Struggled with footwork drills today"

---

### US-03.06: Trainer Segments and Filters Players

**As a** Trainer  
**I want to** filter players by various criteria  
**So that** I can target specific groups for communications or events

**Acceptance Criteria**:
- [ ] Filter panel on left side or top of player list
- [ ] Filter criteria (multi-select, combine filters):
  - **Skill Level**: Beginner, Intermediate, Advanced, Elite
  - **Age**: Age ranges (5-7, 8-10, 11-13, 14-16, 17+) or custom
  - **Gender**: All, Male, Female, Other
  - **Labels**: Select one or more labels
  - **Flags**: Select one or more flags
  - **Team/School/Club**: Dropdown or search
  - **Attendance History**:
    - Attended > X events (number input)
    - Attended in last Y days (number input)
    - Attendance rate > Z% (slider 0-100%)
    - No-shows > N (number input)
  - **Registration Date**: Date range picker
  - **Last Activity**: Active (within 30 days), Inactive (30-90 days), Churned (>90 days)
- [ ] Apply filters → Player list updates to show matching players
- [ ] Count shown: "15 players match your filters"
- [ ] Clear all filters button
- [ ] Active filters shown as removable badges

**Use Cases**:
- Filter: "Beginners with Attendance Rate >80%" → Target for advanced group promotion
- Filter: "Attendance Risk flag + Last Activity >30 days" → Re-engagement campaign
- Filter: "Elite Squad label" → Invite to private event
- Filter: "Scholarship flag" → Export for financial aid communications

---

### US-03.07: Trainer Views Player Detail

**As a** Trainer  
**I want to** view comprehensive player information  
**So that** I can understand player history and status

**Acceptance Criteria**:
- [ ] Click player from list → Player detail page/modal opens
- [ ] **Basic Profile Section**:
  - Name, photo, age, gender
  - Skill level
  - Email, phone
  - School, team, club
  - Parent information (if child account)
  - Best Times / Availability (from Epic-01)
- [ ] **Labels Section**:
  - All applied labels as colored badges
  - "+ Add Label" button
  - Click label to remove
- [ ] **Flags Section**:
  - All applied flags as badges/icons
  - Flag details: Who applied, when, note
  - "+ Add Flag" button
  - Click flag to view details or remove
- [ ] **Notes Section**:
  - Chronological list of general notes
  - "+ Add Note" button
  - Each note shows: Text, author, timestamp
- [ ] **Event History Section**:
  - All events attended with this trainer
  - Columns: Date, Event Title, Attendance Status (Present, Absent, Late, Excused)
  - Per-event notes (if any)
  - Attendance rate: "18 of 20 events (90%)"
  - No-show count: "2 no-shows"
  - Last event attended: Date
- [ ] **Coach Feedback Section** (optional MVP):
  - Notes left by coaches during events
  - Read-only for trainer
  - Coach name, date, event, feedback text

**Actions Available**:
- [ ] Edit player profile (limited fields: skill level, notes)
- [ ] Add label, add flag, add note
- [ ] View full event history (paginated if >50 events)

---

### US-03.08: Trainer Views Quick View Dashboard

**As a** Trainer  
**I want to** see key metrics at a glance  
**So that** I can understand my business performance

**Acceptance Criteria**:
- [ ] Navigate to "Dashboard" or "Quick View"
- [ ] Dashboard shows:

**1. Events This Week**:
- [ ] Count of events scheduled this week
- [ ] Comparison to last week: "+3 vs last week"

**2. RSVPs**:
- [ ] Total RSVPs this week
- [ ] Breakdown by event type (Training, Private, Small Group)

**3. Top Players** ✅ CRITICAL:
- [ ] List of Top 10 players
- [ ] Ranked by: **Number of events attended in last 90 days**
- [ ] Display: Player name, attendance count (e.g., "John Smith - 18 events")
- [ ] Click player → Opens player detail

**4. Flag Alerts**:
- [ ] Count of flagged players by flag type:
  - Behavior: 2
  - High no-show: 3
  - Injured: 1
  - Medical restriction: 0
  - Scholarship: 5
  - Financial aid: 2
  - Contact priority: 4
  - Attendance risk: 3
- [ ] Click flag type → Filters player list by flag

**5. ShareLink Tracking**:
- [ ] Count of link opens this week
- [ ] New players joined this week (via ShareLink)

**6. Attendance Trends** (optional MVP):
- [ ] Average attendance rate this week vs last week
- [ ] No-show rate this week

**Refresh**:
- [ ] Dashboard data updates in real-time or on page refresh
- [ ] Metrics calculated from database (no manual entry)

---

### US-03.09: Coach Views Assigned Players

**As a** Coach  
**I want to** view players assigned to my events  
**So that** I can prepare and provide feedback

**Acceptance Criteria**:
- [ ] Navigate to "Players" (Coach view)
- [ ] List shows players who have RSVP'd to events where coach is assigned
- [ ] NOT shown: All trainer's players (only assigned players)
- [ ] Each player row shows:
  - Name, photo
  - Skill level
  - Attendance summary: "12 events with you"
- [ ] Click player → Player detail view (coach-scoped)

**Player Detail (Coach View)**:
- [ ] Basic info: Skill level, age
- [ ] Attendance history: Events with this coach only
  - Date, event, attendance status
  - Attendance rate with this coach
- [ ] Flags and labels (read-only)
- [ ] Coach feedback: Past feedback from this coach
- [ ] "+ Add Event Feedback" button (see US-03.11)

**Limitations**:
- [ ] Coach cannot see players outside their assigned events
- [ ] Coach cannot edit player profiles (read-only except feedback)
- [ ] Coach cannot remove labels/flags (read-only)

---

### US-03.10: Coach Adds Session Feedback

**As a** Coach  
**I want to** add feedback for players after sessions  
**So that** trainers and parents can see player progress

**Acceptance Criteria**:
- [ ] From player detail (coach view), click "+ Add Session Feedback"
- [ ] Modal opens: Add Feedback
- [ ] Select session: Dropdown of coach's recent sessions with this player
- [ ] Text area: Feedback (up to 500 characters)
- [ ] Save → Feedback added
- [ ] Feedback visible to:
  - Trainer (in player detail, session history)
  - Coach (own feedback)
  - Player/Parent (optional MVP - may defer to Phase 2)

**Feedback Display**:
- [ ] In player detail (trainer view): "[Session Title] - [Date]: [Feedback]"
- [ ] In session history: Linked to specific session
- [ ] Coach name shown: "Coach Mike: 'Great improvement on footwork.'"

**Edit/Delete**:
- [ ] Coach can edit own feedback within 24 hours
- [ ] Coach can delete own feedback within 24 hours
- [ ] Trainer cannot edit coach feedback (read-only)
- [ ] Super Admin can edit/delete any feedback

**Use Cases**:
- "Struggled with left-hand dribbling today. Needs practice."
- "Excellent focus and effort. Ready for advanced drills."
- "Arrived late but caught up quickly."

---

### US-03.11: Coach Invites Player via ShareLink

**As a** Coach  
**I want to** invite players to the trainer's organization  
**So that** I can expand the player base

**Acceptance Criteria**:
- [ ] From coach's player list, click "Invite Player"
- [ ] System generates unique ShareLink for this invitation
- [ ] Link format: `https://app.platform.com/invite/[unique-code]`
- [ ] Coach can copy link or send via email (if integrated)
- [ ] When player clicks link:
  - If not authenticated: Redirect to login
  - After login: Associate player with trainer's organization
  - Player appears in trainer's CRM
- [ ] Link open tracked in metrics (visible to trainer)
- [ ] Coach can view sent invitations and status (pending, accepted)

**Limitations**:
- [ ] Coach cannot apply labels/flags to invited players (only trainer can)
- [ ] Coach can only view invited players in their assigned sessions

---

### US-03.12: Super Admin Views System-Wide CRM

**As a** Super Admin  
**I want to** view all players across all trainers  
**So that** I can monitor platform activity and support trainers

**Acceptance Criteria**:
- [ ] Navigate to "CRM Master" or "All Players"
- [ ] List shows all players from all trainers
- [ ] Tool-specific search: Search by player name, trainer name, email
- [ ] Filters:
  - Trainer (dropdown/autocomplete)
  - Skill level, age, gender
  - Flags
  - Registration date range
  - Last activity
- [ ] Click player → Player detail (Super Admin view)
- [ ] Actions:
  - Apply flags across trainers
  - View player history across trainers (if player associated with multiple)

**Quick View Dashboard (Super Admin)**:
- [ ] System-wide metrics:
  - Total players across all trainers
  - Sessions this week (all trainers)
  - Top Players (system-wide, most active across platform)
  - Flag counts (system-wide)
  - Revenue (system-wide) - see D-ARCH-001, link to Stripe
- [ ] Drill-down by trainer:
  - Select trainer → View trainer-specific metrics

**Permissions**:
- [ ] Super Admin can view/edit all player data
- [ ] Super Admin can apply/remove flags across trainers
- [ ] Super Admin cannot edit trainer-specific labels (respect trainer customization)

---

### US-03.13: Trainer Invites Players via ShareLink

**As a** Trainer  
**I want to** invite players and parents to my organization  
**So that** I can grow my player base

**Acceptance Criteria - Static Mass Invite Link**:
- [ ] Navigate to CRM Tools → "Invite Players"
- [ ] System shows static mass invite link (one per trainer, reusable)
- [ ] Link format: `https://app.platform.com/join/[trainer-unique-code]`
- [ ] Trainer can:
  - Copy link
  - Share via email, SMS, social media
  - Embed in website
- [ ] When player clicks link:
  - If not authenticated: Redirect to login/signup
  - After login: Associate account with trainer's organization
  - Player appears in trainer's CRM

**Acceptance Criteria - Unique Invite Links** (Optional MVP):
- [ ] Trainer can generate unique one-time links per player/parent
- [ ] Use case: Track who invited whom, personalized invitations
- [ ] Link tracked: Who opened, when, if joined

**ShareLink Tracking**:
- [ ] Track link opens (clicks)
- [ ] Track successful joins (player associated)
- [ ] Display in Quick View dashboard:
  - "15 link opens this week"
  - "5 new players joined via ShareLink"
- [ ] Detailed tracking report (optional MVP):
  - Date, link type (static or unique), opens, joins

---

## 8. Data Requirements

### What Information Needs to Be Stored

**For Player Profiles** (extends Epic-01):
- Player unique identifier
- User account reference (parent if child)
- Trainer association (which trainer's organization)
- Skill level (updated by trainer)
- School, team, club
- Registration date (when associated with trainer)
- Last activity timestamp
- Active/inactive status

**For Labels**:
- Label unique identifier
- Trainer who created it
- Label name (up to 50 characters)
- Color code (hex value)
- Created timestamp

**For Player-Label Associations**:
- Which player
- Which label
- Applied by (trainer)
- Applied timestamp

**For Flags**:
- Flag type (enum: 8 system-defined flags)
- Applied to which player
- Applied by (trainer or coach)
- Applied timestamp
- Optional note (why flag was applied)
- Status (active, resolved)
- Resolved by (if resolved)
- Resolved timestamp

**For Notes**:
- Note unique identifier
- Which player
- Note type (general or session-specific)
- If session-specific: Which session
- Note text (up to 1000 characters)
- Created by (trainer or coach)
- Created timestamp
- Last edited timestamp
- Edited by

**For ShareLink Invitations**:
- Link unique code
- Link type (static mass or unique)
- Created by (trainer or coach)
- Created for (which trainer's organization)
- If unique: Intended recipient (email or name)
- Created timestamp
- Opens count
- Last opened timestamp
- Joined status (boolean)
- Joined timestamp

**For Player-Trainer Associations**:
- Which player
- Which trainer
- Association source (sharelink, event_registration, coach_invite)
- If sharelink: Which sharelink
- Associated timestamp

**For Quick View Metrics** (calculated, not stored):
- Top Players ranking (query last 90 days attendance)
- Flag counts (query active flags)
- Session counts (query current week)
- RSVP counts (query current week)
- ShareLink tracking (query link opens)

---

## 9. Business Rules & Logic

### Player Association Rules

**How Players Get Associated with Trainer**:
1. **ShareLink Invitation**: Player clicks link, logs in, associated
2. **Event Registration**: Player RSVPs to trainer's event, associated
3. **Coach Invitation**: Coach invites player, associated with trainer's org

**Multi-Trainer Associations**:
- Player can be associated with multiple trainers
- Each trainer sees only their own association in CRM
- Super Admin sees all associations

### Label Rules

**Creation**:
- Trainer can create unlimited labels
- Label names must be unique within trainer's organization
- Label names are case-insensitive ("Elite" same as "elite")

**Application**:
- Player can have multiple labels
- Labels are trainer-specific (Trainer A's "Elite" ≠ Trainer B's "Elite")
- Removing label does not delete player

**Deletion**:
- Deleting label removes it from all players
- Confirmation required: "Remove [Label] from [N] players?"

### Flag Rules

**System-Defined Flags** (8 flags, cannot add custom):
1. Behavior
2. High no-show rate
3. Injured
4. Medical restriction
5. Scholarship
6. Financial aid
7. Contact priority
8. Attendance risk

**Application**:
- Player can have multiple flags active
- Flag requires optional note (recommended but not required)
- Flag application logged (who, when, note)

**Resolution**:
- Flags can be marked "Resolved"
- Resolved flags hidden from active view but kept in history
- Trainer can reapply same flag if issue recurs

**Auto-Flagging** (Future):
- "High no-show rate": Auto-flag if no-show >3 times in 30 days (defer to Phase 2)
- "Attendance risk": Auto-flag if no activity in 60 days (defer to Phase 2)

### Notes Rules

**General Notes**:
- Added by trainer, visible to trainer and coaches
- Not tied to specific session
- Chronological display (most recent first)
- No character limit for MVP (reasonable limit: 1000 chars)

**Per-Session Notes**:
- Added by coach during/after session
- Tied to specific session in player history
- Trainer can view (read-only)
- Player/Parent can view (optional MVP, may defer to Phase 2)

**Edit Rules**:
- Creator can edit own notes within 24 hours
- After 24 hours: Read-only (contact Super Admin to edit)
- Trainer cannot edit coach notes (read-only)
- Super Admin can edit any notes anytime

### Segmentation Rules

**Filter Combination**:
- Filters are combined with AND logic
- Example: "Skill: Beginner" AND "Attendance Rate >80%" AND "Label: Elite Squad"
- Result: Players matching ALL criteria

**Attendance Filters**:
- "Attended > X sessions": Total sessions with trainer
- "Attended in last Y days": Sessions within date range
- "Attendance rate > Z%": Percentage of registered sessions attended
- "No-shows > N": Count of Absent status

**Empty Results**:
- If no players match filters: "No players match your filters. Try adjusting criteria."

### Top Players Algorithm

**Ranking Logic** ✅ CONFIRMED:
- Count sessions attended in last 90 days (from today)
- Highest attendance count = #1 Top Player
- Ties: Alphabetical by player name

**Attendance Status Counted**:
- **Present**: Counts toward total
- **Late**: Counts toward total (attended)
- **Absent**: Does NOT count
- **Excused**: Does NOT count

**Display**:
- Top 10 players in Quick View dashboard
- Show player name + session count: "John Smith - 18 sessions"
- Click player → Opens player detail

**Edge Cases**:
- If <10 players: Show all players
- If player has 0 sessions in 90 days: Not shown in Top Players

### ShareLink Tracking Rules

**Static Mass Invite Link**:
- One per trainer (reusable)
- Unlimited uses
- Track total opens (clicks)
- Track successful joins (associations created)

**Unique Invite Links** (Optional MVP):
- Generated per invitation
- One-time or multi-use (trainer preference)
- Track opens per link
- Track who joined via which link

**Opening Tracking**:
- Log each link click (timestamp, IP optional)
- Count opens per link
- Display in Quick View dashboard

**Join Tracking**:
- When player associates with trainer via link: Mark link as "Joined"
- Display joined count in Quick View

---

## 10. User Flows

### Flow 1: Trainer Segments Players and Creates Private Event

1. Trainer logs in to CRM Tools
2. Click "Players" → Player list view
3. Open filter panel
4. Apply filters:
   - Skill Level: "Advanced"
   - Label: "Elite Squad"
   - Attendance Rate: >85%
5. Click "Apply Filters"
6. Player list updates: "8 players match your filters"
7. Click "Create Private Event from Segment" (optional feature) OR manually note player names
8. Navigate to Event Builder (Epic-02)
9. Create event, set Visibility: "Private / Invite Only"
10. Select 8 players from filtered list
11. Save event → Private event created for this segment
12. Event visible only to 8 selected players

### Flow 2: Trainer Applies Label and Flag to New Player

1. New player "Sarah Johnson" joins via ShareLink
2. Trainer receives notification: "New player joined"
3. Trainer navigates to CRM → Player list
4. Sees Sarah Johnson at top (most recent)
5. Click Sarah → Player detail opens
6. Review profile: Age 12, no skill level set
7. Click "Edit Profile" → Set Skill Level: "Beginner"
8. Click "+ Add Label" → Select "Summer Camp 2026"
9. Click "+ Add Flag" → Select "Scholarship"
10. Add note: "Referred by Coach Mike. Eligible for financial aid."
11. Save → Label, flag, and note applied
12. Sarah now appears in filtered lists for "Beginners" and "Scholarship"

### Flow 3: Coach Adds Session Feedback After Training

1. Coach "Mike" completes training session: "Basketball Skills - Monday 6 PM"
2. Navigate to "Players" (Coach view)
3. See list of players from today's session
4. Click player "John Smith" → Player detail (coach view)
5. Click "+ Add Session Feedback"
6. Modal opens: Add Feedback
7. Session auto-selected: "Basketball Skills - Monday 6 PM"
8. Enter feedback: "John struggled with left-hand dribbling. Showed great effort. Recommend extra practice."
9. Save → Feedback added
10. Feedback visible to:
    - Trainer (in John's session history)
    - Coach Mike (own feedback view)
    - John's parent (optional MVP)

### Flow 4: Trainer Views Quick View Dashboard and Checks Top Players

1. Trainer logs in to dashboard
2. Navigate to "Quick View" (or default landing page)
3. Dashboard displays:
   - **Sessions This Week**: 8 sessions (+2 vs last week)
   - **RSVPs**: 45 RSVPs (Training: 30, Private: 10, Small Group: 5)
   - **Top Players**:
     1. John Smith - 18 sessions
     2. Sarah Johnson - 15 sessions
     3. Mike Lee - 14 sessions
     4. ...
   - **Flag Alerts**:
     - Attendance risk: 3 players
     - High no-show: 2 players
     - Injured: 1 player
   - **ShareLink Tracking**: 8 link opens, 2 new players joined this week
4. Trainer clicks "John Smith" (Top Player #1)
5. Player detail opens → Review John's attendance, labels, flags
6. Trainer decides to invite John to private advanced session
7. Return to dashboard
8. Click "Attendance risk: 3 players"
9. Player list filtered by "Attendance risk" flag
10. Trainer reviews at-risk players, plans outreach

### Flow 5: Coach Invites New Player via ShareLink

1. Coach "Sarah" meets parent at local event
2. Parent interested in joining trainer's program
3. Coach navigates to "Players" (Coach view)
4. Click "Invite Player"
5. System generates unique ShareLink: `https://app.platform.com/invite/abc123xyz`
6. Coach copies link
7. Coach sends link via text message to parent
8. Parent clicks link on phone
9. Redirect to platform login page
10. Parent signs up (or logs in if existing account)
11. After login: "You're now part of [Trainer Name]'s organization!"
12. Parent account associated with trainer
13. Trainer receives notification: "New player joined via Coach Sarah's invitation"
14. Player appears in trainer's CRM
15. Coach can now see player in "Players" after player RSVPs to coach's session

---

## 11. Performance & Scale Targets

**Response Times**:
- Player list load (500 players): <2 seconds
- Player detail view: <1 second
- Search results: <500ms (real-time)
- Filter application: <1 second
- Quick View dashboard load: <2 seconds

**Note**: Specific technical implementation (database indexing, caching, pagination) decided by developer.

---

## 12. Questions / Open Issues

| ID | Question | Priority | Status | Owner |
|:---|:---|:---:|:---|:---|
| Q-03.01 | Can players/parents view coach feedback on their profiles? (Read-only display) | P2 | Open | Client |
| Q-03.04 | Auto-flag logic: "High no-show" if >3 no-shows in 30 days? MVP or Phase 2? | P2 | Open | Team |
| Q-03.05 | Can coaches apply flags to assigned players, or only trainers? | P2 | Open | Client |
| Q-03.07 | Label usage analytics: "This label is on 12 players, 3 haven't attended in 60 days"? | P2 | Open | Team |

---

## 13. Acceptance Criteria (Epic-Level)

This Epic is complete when:

**Player Management (Trainer)**:
- [ ] Trainer can view all players in organization (list + detail)
- [ ] Trainer can search players by name, email, team, school, attendance
- [ ] Trainer can create and apply custom labels
- [ ] Trainer can apply 8 system-defined flags with notes
- [ ] Trainer can add general notes and per-session notes
- [ ] Trainer can segment/filter players by multiple criteria

**Quick View Dashboard**:
- [ ] Dashboard shows sessions this week, RSVPs, Top Players, flag alerts, ShareLink tracking
- [ ] Top Players algorithm works correctly (highest attendance in 90 days)
- [ ] Flag counts accurate and updated in real-time
- [ ] Dashboard loads in <2 seconds

**Coach Experience**:
- [ ] Coach can view assigned players only
- [ ] Coach can add session feedback for players
- [ ] Coach can invite players via ShareLink (unique links)
- [ ] Coach feedback visible to trainer (read-only)

**Super Admin**:
- [ ] Super Admin can view all players across all trainers
- [ ] Super Admin can search and filter system-wide
- [ ] Super Admin can apply/remove flags across trainers
- [ ] Super Admin can view system-wide Quick View dashboard

**ShareLink System**:
- [ ] Trainer can generate static mass invite link
- [ ] Coach can generate unique invite links
- [ ] Link opens tracked in metrics
- [ ] Successful joins tracked and displayed in dashboard

**Data Integrity**:
- [ ] Labels correctly associated with players (multi-label support)
- [ ] Flags correctly applied and removable (with history)
- [ ] Notes chronological and editable within time window
- [ ] Player-trainer associations tracked by source
- [ ] Segmentation filters work correctly (AND logic)

**Performance**:
- [ ] Player list loads in <2 seconds for 500 players
- [ ] Search results in <500ms
- [ ] Quick View dashboard loads in <2 seconds

**Approval**:
- [ ] Demo approved
- [ ] All P0 and P1 questions resolved
- [ ] Label and flag systems validated
- [ ] Top Players algorithm confirmed

---

## 15. Mockups / Design References

**Key Screens to Design**:
1. CRM Tools - Player list view (trainer)
2. CRM Tools - Player detail view
3. CRM Tools - Filter panel (segmentation)
4. CRM Tools - Create/manage labels modal
5. CRM Tools - Apply flag modal
6. CRM Tools - Add note interface
7. Quick View Dashboard (trainer)
8. Quick View Dashboard (Super Admin - system-wide)
9. Player Profiles (coach view)
10. Add Session Feedback modal (coach)
11. ShareLink invitation interface
12. Export options modal

---

## 16. Testing Considerations

**What Should Be Tested**:

**Functional Testing**:
- All user stories and acceptance criteria met
- All business rules enforced correctly
- All filters and segmentation logic working

**Key Scenarios to Test**:
- Trainer creates labels, applies to players, filters by labels
- Trainer applies flags, removes flags, flag history preserved
- Trainer segments players by multiple criteria (AND logic)
- Coach adds session feedback, visible to trainer
- Coach invites player via ShareLink, player associates with trainer
- Top Players algorithm calculates correctly (90 days, highest attendance)
- Quick View dashboard displays accurate metrics
- ShareLink tracking logs opens and joins

**Edge Cases**:
- Player with 0 sessions (not in Top Players)
- Player with multiple labels and flags
- Filter combination returns 0 players (empty results)
- Coach views player with no session history with coach
- Label deleted (removed from all players)
- Flag resolved (hidden from active view, kept in history)
- Note edited after 24-hour window (blocked)
- ShareLink opened multiple times (count increments)

**Security Testing**:
- Trainer cannot see other trainers' players
- Coach cannot see players outside assigned sessions
- Trainer cannot edit coach notes (read-only)
- Super Admin can see all data (permissions verified)

**Performance Testing**:
- Player list load with 5,000 players
- Filter application with complex criteria
- Top Players calculation with 10,000+ system-wide players
- Quick View dashboard load with large datasets

**Note**: Developer chooses specific testing tools and frameworks.

---

## 17. Implementation Notes

**Suggested Implementation Order**:
1. Player list view and search
2. Player detail view
3. Labels system (create, apply, filter)
4. Flags system (apply, remove, filter)
5. Notes system (general + per-session)
6. Segmentation and filtering (multi-criteria)
7. Quick View dashboard and Top Players algorithm
8. ShareLink invitation and tracking
9. Coach player view and feedback
10. Super Admin system-wide CRM

**Integration Points with Other Epics**:
- **Epic-01** (User Management): Player profiles, parent/child accounts, Best Times integration
- **Epic-02** (Event Management): Attendance data for session history, segmentation filters
- **Epic-04** (LPPP): Can assign content to player segments (future)
- **Epic-06** (Marketing): ShareLink tracking

**Security Requirements** (High-Level):
- Validate user permissions on all CRM operations
- Prevent cross-trainer data access (trainers see only own players)
- Sanitize note input (prevent injection attacks)
- Log flag application/removal (audit trail)
- Rate limit ShareLink generation (prevent spam)

**Notification Requirements**:
- Trainer notified when player joins via ShareLink
- Trainer notified when coach adds feedback
- Coach notified when invited player joins (optional)
- Flag alerts in dashboard (visual indicators)

*Note: Specific email service and notification system chosen by developer*

---

**Priority**: P0 - Core Platform Value (enables player management and business intelligence)
**Complexity**: High (segmentation, Top Players algorithm, ShareLink tracking, coach/trainer permissions)

**Note**: This specification focuses on **business requirements** and **user needs**. Technical implementation details (database schema, API design, caching strategies, specific technologies) are decided by the development team based on their expertise.

