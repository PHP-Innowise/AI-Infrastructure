# Epic-07: Super Admin & System Management

---

## 1. Epic Overview

### Purpose
Provide Super Admin with comprehensive tools to manage the entire platform, including user management, trainer configuration, system monitoring, and operational oversight.

### Business Value
- **For Super Admin**: Efficient platform operations, trainer support, system monitoring
- **For Trainers**: Quick support resolution via impersonation
- **For Platform**: Scalable administration, audit trails, feature rollout control

### Success Metrics
- ✅ <5 minute average time to resolve trainer support issues (via impersonation)
- ✅ 90%+ of trainer configurations completed in <10 minutes
- ✅ Zero unauthorized access incidents (audit logging effective)
- ✅ Dashboard load <2 seconds with 100+ trainers

---

## 2. Scope Summary

**Goals**:
- Operational analytics dashboard
- Trainer account creation and management
- Feature toggles per trainer
- Event Master Tool (system-wide event management)
- User management tools
- Audit logging

**Non-Goals** (Post-MVP):
- Financial reporting (Stripe handles this)
- System health monitoring (storage, errors) - use infrastructure tools
- Bulk operations (one-by-one for MVP)
- Advanced analytics (drill-down, custom reports)
- Email/SMS campaigns from admin panel
- Content moderation tools
- Automated trainer onboarding workflows
- Portal branding UI for Super Admin (trainers configure their own branding; Super Admin uses impersonation to help if needed)

---

## 3. In Scope (MVP)

### Dashboard & Analytics
- [ ] Operational metrics dashboard (user counts, session stats, growth trends)
- [ ] Link to Stripe Dashboard for all financial data
- [ ] No financial data duplicated on platform

### Trainer Management
- [ ] Create trainer accounts (covered in Epic-01, referenced here)
- [ ] View all trainers list
- [ ] Edit trainer details
- [ ] Deactivate/reactivate trainers
- [ ] View trainer subscription status (via Stripe)

### System Configuration
- [ ] **Feature toggles per trainer** (3 features: LPPP, Marketing, Camps)
- [ ] Pricing configuration per trainer (covered in Epic-05, referenced here)

### Tools
- [ ] **Event Master Tool** (view/manage all events system-wide)
- [ ] **Users Tool** (view/edit all users)
- [ ] **Impersonation** (covered in Epic-01, referenced here)
- [ ] **Audit Log** (track critical Super Admin actions)

---

## 4. Out of Scope (Post-MVP)

**Phase 2 Deferrals**:
- ❌ Financial metrics on dashboard (Stripe handles)
- ❌ System health monitoring (storage usage, error rates) - use infrastructure tools
- ❌ Bulk operations (activate 10 trainers at once)
- ❌ Advanced analytics (drill-down, custom date ranges beyond presets)
- ❌ User Role Editor (custom role creation)
- ❌ Content moderation (flag/review public LPPP content)
- ❌ Email/SMS campaign creation from admin panel
- ❌ Trainer onboarding workflows (checklists, automated emails)
- ❌ Platform-wide announcements
- ❌ System backup/restore UI

---

## 5. Dependencies

### Required Before This Epic
- **Epic-01** (User Management) - Impersonation, user accounts
- **Epic-02** (Event Management) - Events to display in Event Master Tool
- **Epic-05** (Payments) - Stripe integration for financial links

### Blocks These Epics
- None (can be developed in parallel with Epic-06)

### External Dependencies
- **Stripe Dashboard**: All financial data viewed externally

---

## 6. User Roles Involved

| Role | Interaction | Permissions |
|:---|:---|:---|
| **Super Admin** | Full system access, management, configuration | Can view/edit all data, configure all settings, impersonate users, view audit logs |

*Note*: Only Super Admin role interacts with this Epic. All tools are Super Admin-only.

---

## 7. User Stories & Acceptance Criteria

### US-07.01: Super Admin Views Operational Dashboard

**As a** Super Admin  
**I want to** see platform usage and engagement metrics  
**So that** I can monitor platform health and growth

**Acceptance Criteria**:
- [ ] After login, Super Admin sees dashboard (or navigate to "Dashboard")
- [ ] Dashboard shows **operational metrics only** (no financial data)

**User Metrics**:
- [ ] Total trainers (active subscriptions via Stripe)
- [ ] Total players (registered)
- [ ] Total coaches (registered)
- [ ] New users this week
- [ ] New users this month
- [ ] User growth chart (last 30 days)

**Session Metrics**:
- [ ] Sessions this week (training sessions, private sessions, camps)
- [ ] RSVPs this week
- [ ] Attendance rate (%)
- [ ] No-show rate (%)

**Top Performers**:
- [ ] Most active trainers (by session count or player count)
  - Trainer name, session count, player count
- [ ] Top players (by attendance)
  - Player name, trainer, attendance count

**Date Range Selector**:
- [ ] Presets: Last 7 days, Last 30 days, Last 90 days
- [ ] Default: Last 30 days

**Financial Data** → **Stripe Dashboard**:
- [ ] Prominent button: "View Financial Reports in Stripe"
- [ ] Clicks → Opens Stripe Express Dashboard (Super Admin's account)
- [ ] NO financial metrics on platform dashboard (revenue, payouts, transactions)

**Rationale**:
- Stripe is financial source of truth
- Platform focuses on operational and engagement metrics Stripe can't provide
- Avoids duplicate reporting and potential conflicts

---

### US-07.02: Super Admin Views All Users

**As a** Super Admin  
**I want to** search and view all users across all trainers  
**So that** I can provide support and manage accounts

**Acceptance Criteria**:
- [ ] Navigate to "Users" tool
- [ ] View list of all users (trainers, coaches, players, parents)
- [ ] **Tool-specific search**:
  - Search box: "Search users by name or email"
  - Filters: Role (All, Trainer, Coach, Player), Status (Active, Inactive)
  - Results shown in table

**User Table Shows**:
- [ ] Name
- [ ] Email
- [ ] Role (Trainer, Coach, Player/Parent)
- [ ] Associated trainer (if coach/player)
- [ ] Status (Active, Inactive, Pending)
- [ ] Registration date
- [ ] Last login

**Actions Per User**:
- [ ] View profile (click name → user details page)
- [ ] Edit user (button)
- [ ] Impersonate user (button) - see Epic-01 US-01.07
- [ ] Deactivate user (button) - see Epic-01 US-01.12

**Pagination**:
- [ ] Show 50 users per page
- [ ] Navigate between pages

---

### US-07.03: Super Admin Edits User Profile

**As a** Super Admin  
**I want to** directly edit user profiles  
**So that** I can fix issues quickly without impersonating

**Acceptance Criteria**:
- [ ] From Users list, click "Edit" on user row
- [ ] Edit form opens with user details:
  - Name (editable)
  - Email (editable, with warning: "Changing email requires re-verification")
  - Role (view-only, cannot change role directly)
  - Status (Active / Inactive toggle)
  - Associated trainer (view-only for coaches/players)
  - Profile details (age, gender, etc. if player)
- [ ] Save changes
- [ ] User profile updated
- [ ] Audit log created: "Super Admin edited user [Name]"

**Use Cases**:
- Fix typo in player name
- Update parent email
- Correct player age/grade

**Note**: For complex changes (role changes, account merges), use impersonation instead

---

### US-07.04: Super Admin Creates Trainer Account

**As a** Super Admin  
**I want to** create new trainer accounts  
**So that** I can onboard trainers to the platform

**Acceptance Criteria**:
- [ ] **Reference**: See Epic-01 US-01.01 for full specification
- [ ] From Users tool, click "Create Trainer"
- [ ] Fill form: Business name, trainer name, email, subscription tier
- [ ] Save → Trainer account created
- [ ] Invitation email sent to trainer with onboarding instructions
- [ ] Trainer receives login credentials
- [ ] Stripe subscription created (via Epic-05 integration)

**Dashboard Link**:
- [ ] Dashboard shows: "Create Trainer" quick action button

---

### US-07.05: Super Admin Configures Feature Toggle per Trainer

**As a** Super Admin  
**I want to** enable or disable specific features for individual trainers  
**So that** I can control feature rollout and customize per trainer

**Acceptance Criteria**:
- [ ] Navigate to "Trainers" list
- [ ] Select trainer → "Feature Settings"
- [ ] View feature toggle list:

**Feature Toggles** (3 for MVP):
1. **LPPP Content System** (Epic-04)
   - Toggle: ON / OFF
   - Description: "Enable Learn, Practice, Perfect content creation and player access"
   - Default: ON

2. **Marketing Tools** (Epic-06)
   - Toggle: ON / OFF
   - Description: "Enable referrals and coupon codes"
   - Default: ON

3. **Camps** (Epic-02 - if in scope, Q-07.01)
   - Toggle: ON / OFF
   - Description: "Enable camp events with external registration forms"
   - Default: ON (if feature exists)

**Toggle Actions**:
- [ ] Click toggle to enable/disable
- [ ] Confirmation: "Disable LPPP Content for [Trainer]? Existing content will be hidden from players."
- [ ] Save changes
- [ ] Feature immediately enabled/disabled for that trainer
- [ ] Trainer sees/doesn't see feature in their UI

**Use Cases**:
- Beta testing: Enable Marketing Tools for 5 trainers first
- Phased rollout: Enable LPPP for trainers who request it
- Custom plans: Disable features for lower-tier subscriptions (future)

**Audit Logging**:
- [ ] Log: Who toggled, what feature, for which trainer, when

---

### US-07.06: Super Admin Views Event Master Tool

**As a** Super Admin  
**I want to** view and manage all events across all trainers  
**So that** I can resolve scheduling issues and monitor platform activity

**Acceptance Criteria**:
- [ ] Navigate to "Events" or "Event Master"
- [ ] View all events from all trainers (system-wide)

**Event List Shows**:
- [ ] Event title
- [ ] Trainer (who created it)
- [ ] Date & time
- [ ] Event type (Session, Private, Camp)
- [ ] Capacity (e.g., "15 / 20")
- [ ] Status (Upcoming, Completed, Canceled)
- [ ] Coach assigned (if any)

**Filters** (Tool-Specific Search - D-SCOPE-005):
- [ ] Search by event title or trainer name
- [ ] Filter by: Date range (next 7 days, next 30 days, custom)
- [ ] Filter by: Trainer (select from list)
- [ ] Filter by: Event type
- [ ] Filter by: Status

**Actions Per Event**:
- [ ] View details (click event → full event page)
- [ ] Edit event (as if Super Admin created it)
- [ ] Cancel event (emergency override)
- [ ] View RSVP list

**Sort**:
- [ ] Sort by: Date (ascending/descending), Trainer, Capacity

**Pagination**:
- [ ] Show 50 events per page

**Use Cases**:
- Trainer reports: "My event isn't showing"
- Monitor platform activity: See most popular event types
- Emergency cancellation: Weather event, facility closure

---

### US-07.07: Super Admin Overrides Scheduling Conflict

**As a** Super Admin  
**I want to** override scheduling conflict warnings  
**So that** I can resolve urgent issues without restrictions

**Acceptance Criteria**:
- [ ] When Super Admin creates/edits event (via Event Master Tool)
- [ ] If scheduling conflict detected:
  - Example: Coach assigned to 2 events at same time
  - Example: Trainer has event at unavailable time
- [ ] **No warning shown** (Super Admin override - SAQ9)
- [ ] Event saved without confirmation
- [ ] Conflict logged in audit log: "Super Admin overrode conflict: [details]"

**Rationale**:
- Super Admin has full control to resolve edge cases
- Example: Trainer calls: "Emergency, I need to move my event to Coach Sarah's time slot"
- Super Admin can make change immediately without warnings

---

### US-07.08: Super Admin Views Audit Log

**As a** Super Admin  
**I want to** view all critical admin actions  
**So that** I can track changes and ensure accountability

**Acceptance Criteria**:
- [ ] Navigate to "Audit Log" or "Activity Log"
- [ ] View chronological list of logged actions:

**Logged Actions**:
- [ ] Impersonation sessions (who, when, duration) - see Epic-01
- [ ] Trainer account created/deactivated
- [ ] User account deleted (GDPR compliance)
- [ ] Feature toggle changed (what, for whom)
- [ ] Pricing changed (per-trainer rates)
- [ ] Scheduling conflict overridden (which event)

**Log Entry Shows**:
- [ ] Timestamp (date & time)
- [ ] Action performed (e.g., "Impersonated User")
- [ ] Subject (who/what was affected, e.g., "John Smith - Trainer")
- [ ] Details (e.g., "Duration: 15 minutes")
- [ ] Admin who performed action (if multiple Super Admins in future)

**Filters**:
- [ ] Date range (last 7 days, last 30 days, custom)
- [ ] Action type (Impersonation, User Changes, Feature Toggles, etc.)
- [ ] Search by subject (user name, trainer name)

**Export**:
- [ ] "Export to CSV" button for compliance audits

**Retention**:
- [ ] Logs retained for at least 1 year (configurable)

---

### US-07.09: Super Admin Links to Stripe Dashboard

**As a** Super Admin  
**I want to** easily access Stripe Dashboard from platform  
**So that** I can view financial data and manage trainer subscriptions

**Acceptance Criteria**:
- [ ] From platform dashboard, prominent button: "View Financial Reports in Stripe"
- [ ] Click → Opens Stripe Express Dashboard in new tab
- [ ] Auto-login if session active (SSO if possible)
- [ ] Stripe Dashboard shows:
  - Revenue breakdown
  - Transaction history
  - Trainer subscriptions
  - Payout schedules
  - Tax documents
  - Dispute/refund management

**Also Available**:
- [ ] Per-trainer view: "View [Trainer]'s Stripe Account"
  - Opens that trainer's Stripe Connect account
  - Shows trainer's earnings, payouts, subscription status

**Rationale**:
- Stripe is the financial source of truth
- No point duplicating financial reports on platform
- Stripe provides comprehensive tools Super Admin needs

---

## 8. Data Requirements

### What Information Needs to Be Stored

**For Audit Logs**:
- Log entry unique identifier
- Timestamp
- Action type (impersonation, user edit, feature toggle, etc.)
- Subject (user, trainer, event affected)
- Details (JSON or text field with action-specific data)
- Admin who performed action (Super Admin reference)

**For Feature Toggles**:
- Trainer reference
- Feature name (LPPP, Marketing, Camps)
- Status (enabled / disabled)
- Last updated timestamp
- Last updated by (Super Admin reference)

**For Dashboard Metrics** (calculated, not stored):
- User counts (query database)
- Session counts (query events table)
- Growth trends (calculate from user registration dates)
- Top performers (calculate from attendance records)

---

## 9. Business Rules & Logic

### Feature Toggle Rules

**Feature Scope**:
- LPPP: If disabled, trainer cannot access LPPP section, players see no content
- Marketing: If disabled, trainer cannot create coupons or view referral dashboard
- Camps: If disabled, trainer cannot create camp events

**Default State**:
- All features enabled by default for new trainers
- Super Admin can disable before trainer onboarding (e.g., for custom plans)

**Toggle Effect**:
- Immediate: Feature disappears from trainer UI within seconds
- Existing data preserved: Disabling LPPP doesn't delete content, just hides it
- Re-enabling: All previous data reappears

### Audit Logging Rules

**What Gets Logged**:
- All Super Admin actions that modify data
- All impersonation sessions
- Critical configuration changes

**What Does NOT Get Logged** (too verbose):
- Super Admin viewing dashboards
- Super Admin searching users
- Super Admin viewing audit log itself

**Log Retention**:
- Minimum 1 year
- Configurable (can extend for compliance)

### Dashboard Metrics Rules

**User Count Logic**:
- Active trainers: Count of trainers with active Stripe subscriptions
- Total players: Count of player accounts with status = active
- Total coaches: Count of coach accounts with status = active

**Growth Calculation**:
- New users this week: Count of users registered in last 7 days
- Growth trend: Compare current week to previous week (% change)

**Top Performers**:
- Most active trainers: Ranked by session count in last 30 days
- Top players: Ranked by attendance count in last 30 days

---

## 10. User Flows

### Flow 1: Super Admin Creates Trainer and Configures Portal

1. Super Admin logs in
2. Dashboard shows: "Create Trainer" button
3. Click "Create Trainer"
4. Fill form:
   - Business name: "Bob's Basketball Academy"
   - Trainer name: "Coach Bob"
   - Email: bob@example.com
   - Subscription tier: Standard ($15/month)
5. Save
6. System creates trainer account
7. Stripe subscription created
8. Invitation email sent to bob@example.com
9. Super Admin redirected to trainer profile
10. Click "Feature Settings"
11. Enable: LPPP, Marketing
12. Disable: Camps (Bob doesn't need it)
13. Save
14. Bob receives email: "Your the platform account is ready!"
15. Bob logs in to platform
16. Bob only sees LPPP and Marketing tools (no Camps)

### Flow 2: Super Admin Resolves Trainer Support Issue

1. Trainer calls: "Help! I can't see my event from yesterday"
2. Super Admin logs into platform
3. Navigate to "Event Master"
4. Search: Trainer name
5. Filter: Last 7 days
6. Finds event: "Basketball Skills - Yesterday 6 PM"
7. Event status: "Completed" (not "Upcoming")
8. Realizes: Event auto-completed after end time
9. Super Admin calls trainer: "It's in your Completed Events tab"
10. Trainer: "Oh! Found it. Thanks!"
11. Issue resolved in 2 minutes

### Flow 3: Super Admin Monitors Platform Growth

1. Super Admin logs in daily
2. Dashboard shows:
   - Total trainers: 52 (up from 48 last week)
   - Total players: 1,234 (up from 1,150 last week)
   - Sessions this week: 245
   - Attendance rate: 87%
3. Growth chart shows upward trend
4. Top trainers:
   - Coach Bob: 45 sessions, 120 players
   - Coach Lisa: 38 sessions, 95 players
5. Super Admin satisfied, growth on track
6. Clicks "View Financial Reports in Stripe"
7. Stripe Dashboard shows: $18,000 revenue this month
8. All good!

---

## 11. Performance & Scale Targets

**Response Times**:
- Dashboard load: <2 seconds (with 100 trainers)
- User search: <500ms
- Event Master Tool load: <2 seconds (with 1,000 events)
- Audit log load: <1 second (with 10,000 entries)

**Scalability**:
- Support 500 trainers
- Handle 10,000 players
- Display 5,000 events in Event Master Tool (paginated)
- Store 100,000 audit log entries

**Data Refresh**:
- Dashboard metrics: Update hourly (batch job)
- Feature toggles: Immediate effect
- Audit logs: Real-time append

---

## 12. Questions / Open Issues

| ID | Question | Priority | Status | Owner |
|:---|:---|:---:|:---|:---|
| Q-07.01 | **Camps scope**: Are camps in MVP scope? If yes, include in feature toggles. If no, remove toggle. | P1 | ✅ RESOLVED | Super Admin |
| Q-07.02 | Dashboard date range: Are presets (7/30/90 days) sufficient, or need custom date picker? | P2 | Open | Team |
| Q-07.03 | Bulk operations: Should Super Admin be able to deactivate multiple trainers at once? (Assume NO for MVP) | P2 | ✅ RESOLVED | Team |

**Resolutions**:
- **Q-07.01**: YES - Camps are in MVP scope as Epic-08 (Forms & Registration), feature toggle "Camps" included
- **Q-07.03**: NO - Bulk operations deferred to Post-MVP

---

## 13. Integration Points

**With Epic-01 (User Management)**:
- User editing uses same user data model
- Impersonation functionality (US-01.07) referenced from Super Admin tools
- Trainer creation (US-01.01) accessible from Super Admin dashboard

**With Epic-02 (Event Management)**:
- Event Master Tool displays all events
- Super Admin can edit/cancel events

**With Epic-05 (Payments)**:
- Stripe Dashboard links for financial data
- Trainer subscription status visible in trainer list

**With Epic-06 (Marketing)**:
- Feature toggle enables/disables Marketing Tools

**With Epic-04 (LPPP Content)**:
- Feature toggle enables/disables LPPP system

**With Epic-03 (CRM)**:
- **Trainer Quick View Dashboard**: Trainers have their own dashboard (US-03.09 in Epic-03)
  - Shows player metrics, Top Players, attendance trends for that trainer
  - Super Admin dashboard shows system-wide version of similar metrics

---

## 14. Acceptance Criteria (Epic Level)

**Dashboard**:
- [ ] Operational metrics displayed accurately
- [ ] Link to Stripe Dashboard works
- [ ] No financial data on platform dashboard
- [ ] Dashboard loads <2 seconds

**Trainer Management**:
- [ ] Can create trainer accounts
- [ ] Can view/edit trainer details
- [ ] Can deactivate trainers
- [ ] Can configure feature toggles per trainer

**Tools**:
- [ ] Event Master Tool shows all events with filters
- [ ] Users Tool shows all users with search
- [ ] Audit Log tracks critical actions
- [ ] All tools load <2 seconds

**Feature Toggles**:
- [ ] LPPP toggle works (enables/disables content system)
- [ ] Marketing toggle works (enables/disables referrals/coupons)
- [ ] Camps toggle works (if camps in scope)
- [ ] Toggle changes take effect immediately

**Audit Logging**:
- [ ] All critical actions logged
- [ ] Logs viewable with filters
- [ ] Logs exportable to CSV
- [ ] Logs retained for 1 year minimum

---

**Total User Stories**: 9

