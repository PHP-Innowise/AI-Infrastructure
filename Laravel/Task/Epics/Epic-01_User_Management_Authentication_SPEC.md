# Epic-01: User Management & Authentication

---

## 1. Description

**Epic-01** establishes the foundation for the platform by implementing a comprehensive multi-role user management system with authentication, authorization, and role-based access control.

This epic enables the platform to support **four distinct user roles** (Super Admin, Trainer, Coach, Player/Parent) with carefully defined permissions, workflows for user onboarding and management, and critical features like impersonation, soft deletion, and availability management.

**Business Value**: Without proper user management, the platform cannot function. This epic provides:
- Secure authentication and session management
- Clear role separation for different platform users
- Trainer autonomy in managing their organization
- Player/Parent self-service capabilities
- Super Admin oversight and support tools
- Foundation for all other platform features

---

## 2. Business Value

**Problem Statement**:
The platform serves multiple distinct user types (platform owner, trainers running their businesses, coaches delivering sessions, families attending training), each with different needs and permissions. A generic "one size fits all" user system would create security risks, confusing UX, and operational overhead.

**Success Metrics**:
- ✅ 100% of users can access only features appropriate to their role
- ✅ Trainers can onboard players and coaches in <5 minutes via share links
- ✅ Super Admin can impersonate and resolve user issues without code deployments
- ✅ 0% data leakage between trainer organizations
- ✅ <2 seconds page load for user dashboards
- ✅ 95%+ successful self-service parent/child profile creation

---

## 3. In Scope (MVP)

### Core Authentication & Authorization
- [ ] Email/password authentication
- [ ] Role-based access control (4 roles)
- [ ] Session management and security
- [ ] Password reset flow
- [ ] Email verification

### User Management (Super Admin)
- [ ] Users tool - global user directory
- [ ] Create trainer accounts (Super Admin only)
- [ ] Edit user accounts and profiles
- [ ] Deactivate users (soft delete, history preserved)
- [ ] Delete users (PII anonymized, history preserved as "Deleted User")
- [ ] Impersonate users (except other Super Admins)
- [ ] Impersonation audit logging
- [ ] Tool-specific search (not global)

### Player/Parent Features
- [ ] Registration via ShareLink (trainer invitation)
- [ ] **Camp-to-User Conversion** (Integration with Epic-08)
  - After camp/evaluation form submission → prompt to create account
  - Pre-fill registration form with camp submission data
  - Seamless conversion flow (no re-entering information)
  - Auto-assign to trainer after account creation
  - Alternative: Send ShareLink via email for later registration
- [ ] Multi-trainer association support with separated views
- [ ] **Parent registers as player** (parent account can train themselves)
- [ ] Player profile management
- [ ] Parent/child profile relationships
- [ ] Child profile creation with trainer selection
- [ ] **Parent manages child-trainer associations** (add/remove children from trainers)
- [ ] Child purchase approval workflow
- [ ] **Child login with constraints** (limited permissions, cannot add trainers)
- [ ] **ShareLink blocking for children** (parent notification when child clicks new trainer link)
- [ ] Availability settings (when player is available to attend events)
- [ ] **Context switching UI** (parent sees "Me" + "Children", child sees own trainers only)

### Coach Features
- [ ] Coach assignment to single trainer
- [ ] My Times (coach availability management)
- [ ] Availability conflict warnings with override
- [ ] Public profile management (bio, credentials)

### Trainer Features
- [ ] ShareLink generation (static for players, unique for coaches)
- [ ] View player availability (scheduling optimization tool)
- [ ] Assign coaches to events (with availability checks)
- [ ] Manage own organization users
- [ ] Portal branding (logo upload + color selection)

---

## 4. Out of Scope (Post-MVP)

**Phase 2 Deferrals**:
- ❌ Open Gym / League Instructor role
- ❌ Referee / Contractor role
- ❌ Social login (Google, Facebook, Apple)
- ❌ Two-factor authentication (2FA)
- ❌ Advanced permission customization per user
- ❌ Coaches Corner (messaging hub)
- ❌ Build-a-Bag (skill focus tool)
- ❌ Feedback tool (player feedback system)
- ❌ Custom role creation (User Role Editor full version)
- ❌ Advanced portal branding (fonts, full layout customization)
- ❌ User bulk import/export
- ❌ API access for external integrations

**Note**: Simple portal branding (logo upload + color selection) IS included in MVP - see US-01.12

---

## 5. Dependencies

### Required Before This Epic
- **None** - This is the foundation epic

### Blocks These Epics
- **Epic-02** (Event Management) - Requires users to create/attend events
- **Epic-03** (CRM & Player Management) - Requires trainer/player relationships
- **Epic-04** (LPPP Content System) - Requires users to create/consume content
- **Epic-05** (Payments) - Requires user accounts and trainer organizations
- **Epic-06** (Marketing Tools) - Requires user base to market to
- **Epic-07** (Super Admin Tools) - Requires users to manage

### External Dependencies
- **Database System**: Persistent storage for user accounts, profiles, relationships
- **Email Service**: Transactional emails (welcome, password reset, invitations)
- **File Storage**: Profile photos and avatars
- **Session Management**: Secure user sessions and authentication tokens

---

## 6. User Roles Involved

| Role | Interaction | Permissions |
|:---|:---|:---|
| **Super Admin** | Manages entire platform, creates trainers, impersonates users for support, configures system | Full access to all features, can override any rule, cannot impersonate other Super Admins |
| **Trainer / Business Owner** | Manages own training organization, creates events, invites players/coaches, views analytics | Full control within own org, no access to other trainers' data, no system config access |
| **Coach / Contractor** | Delivers training sessions, provides feedback, manages availability | Assigned to ONE trainer at a time, view-only on most features, edit own profile and availability |
| **Player / Parent** | Registers for training, manages profiles, sets availability, pays for sessions | Self-service profile management, can connect to multiple trainers, child actions require parent approval |

---

## 7. User Stories & Acceptance Criteria

### US-01.01: Super Admin Creates Trainer Account

**As a** Super Admin  
**I want to** create new trainer accounts  
**So that** trainers can start using the platform to run their businesses

**Acceptance Criteria**:
- [ ] From Users tool, click "Create User" → Select "Trainer" role
- [ ] Enter: Business name, trainer name, email, phone
- [ ] System generates temporary password OR sends invite email with setup link
- [ ] Trainer receives email with login credentials / setup instructions
- [ ] Trainer can log in and access trainer dashboard
- [ ] New trainer account appears in Users list with status "Active"
- [ ] Validation: Email must be unique, required fields enforced
- [ ] Error handling: Duplicate email shows clear error message

**Implementation Notes**:
- Send professional invitation email using email service
- Temporary password must be changed on first login (force password reset)
- Audit log: Record who created trainer, when, and trainer details

---

### US-01.02: Player Registers via ShareLink

**As a** Player or Parent  
**I want to** register for training via a link from my trainer  
**So that** I can join the program and see available sessions

**Acceptance Criteria**:
- [ ] Click trainer's ShareLink (e.g., `https://app.platform.com/join/ABC123`)
- [ ] If not logged in: Redirect to registration/login page
- [ ] Registration form: Name, email, password, phone (parent), player name/age/gender
- [ ] After registration: Auto-associate with trainer who sent link
- [ ] Player profile created in trainer's CRM
- [ ] Player can now view trainer's events and content
- [ ] Confirmation email sent to player/parent
- [ ] If already logged in: Instant association, redirect to trainer's events

**Acceptance Criteria (Multi-Trainer)**:
- [ ] Player with existing account clicks different trainer's ShareLink
- [ ] System associates player with second trainer (no duplicate account)
- [ ] **If Parent with Children**: Show selection prompt:
  - [ ] "Who will train with [New Trainer]?"
  - [ ] Checklist shows: Parent (Me) + all children
  - [ ] Parent selects applicable family members
  - [ ] Only selected family members associated with new trainer
- [ ] **Separated Views Architecture**:
  - [ ] Player switches between trainer contexts (like switching accounts)
  - [ ] Each context shows completely isolated data: calendar, tokens, content, reservations
  - [ ] No combined/unified view within platform
  - [ ] Context switcher in navigation (e.g., dropdown: "Trainer A" / "Trainer B")
  - [ ] Current trainer context persists across session

**Implementation Notes**:
- ShareLink must be associated with the trainer who created it
- Static links (unlimited uses, no expiry) for players
- Unique links (single use, 7-day expiry) for coaches
- Track ShareLink usage for analytics (Epic-06)

---

### US-01.03: Parent Creates Child Profile

**As a** Parent  
**I want to** create profiles for my children  
**So that** I can manage training for my whole family from one account

**Acceptance Criteria**:
- [ ] From Player Profiles page, click "+ Add Child"
- [ ] Enter: Child name, age, gender, optional (school, photo)
- [ ] Mark as "Child" (vs. "Self")
- [ ] **Trainer Selection**:
  - [ ] If parent has **only ONE trainer**: Prompt "Will [Child] also train with [Trainer]?" (Yes/No)
  - [ ] If parent has **multiple trainers**: Show trainer selection checklist
  - [ ] If "Yes" to single trainer OR trainers selected: Child associated with those trainers
  - [ ] If "No" or none selected: Child profile created but not associated with any trainer yet
- [ ] System links child profile to parent account
- [ ] Parent can switch between children in UI (context selector dropdown)
- [ ] Each child has separate: Training calendar, RSVP status, attendance, availability preferences **per trainer**
- [ ] Child can optionally have separate login (shares parent's contact info, needs approval for purchases)

**Validation**:
- [ ] Required: Name, age, gender
- [ ] Age: 1-18 years (children only, adults use own accounts)
- [ ] Duplicate check: Warn if similar name/age exists

**Important Notes**:
- Parent account is treated as a player account (parent can train themselves)
- Child-trainer associations are explicit (not automatic, except single-trainer prompt)
- Parent can modify child-trainer associations anytime from profile settings

---

### US-01.04: Parent Manages Child-Trainer Associations

**As a** Parent  
**I want to** add or remove my children from trainers  
**So that** I can control which programs each child participates in

**Acceptance Criteria**:
- [ ] Navigate to "Family" or "Player Profiles" section
- [ ] View list of all children with their trainer associations
- [ ] For each child, see: Name, Age, Associated Trainers (with dates)

**Add Child to Trainer**:
- [ ] Click "Add Trainer" for specific child
- [ ] Option A: Enter ShareLink manually
- [ ] Option B: Select from "My Trainers" (trainers parent is already associated with)
- [ ] Confirm association
- [ ] Child now associated with trainer, can see events/content

**Remove Child from Trainer**:
- [ ] Click "Remove" next to child-trainer relationship
- [ ] Confirmation: "Remove [Child] from [Trainer]? This will cancel all upcoming RSVPs."
- [ ] Confirm removal
- [ ] Child disassociated from trainer
- [ ] Child's data with that trainer soft-deleted (history preserved)
- [ ] Trainer no longer sees child in their roster

**Context Switching Documentation**:

**Parent Context Selector** (Parent who also trains):
```
[Current Context: Sarah (Me) → Coach Lisa ▼]

Your Training:
  Sarah (Me) → Coach Lisa
  Sarah (Me) → Coach Mike

Your Children's Training:
  Alex → Coach Bob
  Maya → Coach Bob
  Maya → Coach Lisa
  Emma → Coach Bob
```

**Parent Context Selector** (Parent who doesn't train):
```
[Current Context: Alex → Coach Bob ▼]

Your Children's Training:
  Alex → Coach Bob
  Maya → Coach Bob
  Maya → Coach Lisa
  Emma → Coach Bob
```

**Child Context Selector** (Child with own login):
```
[Current Context: Coach Bob ▼]

Your Training:
  Coach Bob (Basketball)
  Coach Lisa (Volleyball)
```

---

### US-01.05: Child Purchase Requires Parent Approval

**As a** Parent  
**I want to** control my child's spending  
**So that** I can manage family finances and verify training choices

**Acceptance Criteria - USD Payments (Always Require Approval)**:
- [ ] Child (logged in as child) selects event requiring USD payment
- [ ] At checkout, system detects child account → Status: "Pending Parent Approval"
- [ ] Parent receives notification (email + in-app)
- [ ] Parent views pending request in Payments or Reservations section
- [ ] Parent can: Approve (payment processed), Deny (child notified), Request more info
- [ ] After approval: Payment processed, child registered for event
- [ ] Child sees status change from "Pending" to "Confirmed"

**Acceptance Criteria - Token Spending (Optional Approval)**:
- [ ] Parent has setting per child: "Allow token spending without approval" (Default: OFF)
- [ ] **If setting OFF** (Default): Same approval workflow as USD payments
- [ ] **If setting ON**: Child can spend tokens directly without approval
  - Token payment processed immediately
  - Child registered for event instantly
  - Parent receives notification (informational only, not approval request)
- [ ] Parent can change this setting anytime in child's profile settings

**Implementation Notes**:
- System must track: which child, which event, amount, payment type, approval status, timestamps
- Email notification sent to parent when approval needed (USD or tokens if approval enabled)
- Pending requests expire after 48 hours (auto-deny with notification)
- Token approval setting is per-child (different rules for different children)

---

### US-01.06: Child Login with Constraints

**As a** Parent  
**I want** my child to have limited login access  
**So that** they can view their training but cannot make unauthorized changes

**Child CAN Do** (when logged in):
- [ ] Browse eligible events (view-only)
- [ ] RSVP to events (requires parent approval - see US-01.04)
- [ ] Cancel RSVP (requires parent approval)
- [ ] View assigned content (if purchased)
- [ ] View own training progress
- [ ] Submit feedback requests (Perfect pillar)
- [ ] Update basic profile info (photo, preferences)
- [ ] View tokens (view-only, cannot purchase)
- [ ] Switch between trainer contexts (if trains with multiple trainers)

**Child CANNOT Do**:
- [ ] ❌ Add new trainers (ShareLink registration blocked)
- [ ] ❌ Add/remove payment methods
- [ ] ❌ Purchase tokens
- [ ] ❌ Complete purchases without parent approval
- [ ] ❌ Delete their account
- [ ] ❌ Change trainer associations
- [ ] ❌ View parent's training data

**ShareLink Blocking Flow**:
- [ ] Child (logged in) clicks trainer ShareLink
- [ ] System detects: User is a child account
- [ ] Show message: "Ask your parent to register you with this trainer"
- [ ] **Parent Notification**: Send email to parent:
  - Subject: "[Child Name] wants to join [Trainer Name]'s program"
  - Body: ShareLink + "Click to register [Child] with this trainer"
  - CTA button: "Review Registration"
- [ ] Child NOT associated with new trainer (parent must complete registration)

**Context Switching for Children**:
- [ ] If child trains with multiple trainers, show context selector
- [ ] Context selector shows only child's own trainer contexts (no parent data)
- [ ] Format: Simple trainer list (no "Me" section like parents have)

**Open Question** (Q-01.05):
- Should ALL players under 18 require parent accounts?
- Or allow 16-18 year olds to have independent accounts?
- COPPA compliance considerations

---

### US-01.07: Super Admin Impersonates User

**As a** Super Admin  
**I want to** view the platform as any user  
**So that** I can troubleshoot issues and provide support

**Acceptance Criteria**:
- [ ] From Users tool, click "Impersonate" button on user row
- [ ] Confirmation modal: "View platform as [User Name] ([Role])?"
- [ ] After confirm: Portal switches to impersonated user's view
- [ ] Top banner visible (sticky): "Viewing as [User Name] | Exit Impersonation"
- [ ] Banner color-coded (e.g., red/orange) to indicate impersonation mode
- [ ] All navigation, permissions, data matches impersonated user exactly
- [ ] Click "Exit Impersonation" → Return to Super Admin view
- [ ] Impersonation logged: Who impersonated whom, start time, end time, duration

**Security Requirements**:
- [ ] CANNOT impersonate other Super Admin accounts (validation error)
- [ ] All actions during impersonation logged with admin_id context
- [ ] Impersonation session expires after 1 hour (or explicit exit)
- [ ] Audit report available: "Impersonation History" for compliance

---

### US-01.08: Trainer Invites Coach

**As a** Trainer  
**I want to** invite a coach to work with my organization  
**So that** I can delegate session delivery and expand my program

**Acceptance Criteria**:
- [ ] From Coaches section, click "Invite Coach"
- [ ] Enter coach email, optional (name, message)
- [ ] System generates unique ShareLink (one-time use, 7-day expiry)
- [ ] Invitation email sent to coach with link and message
- [ ] Coach clicks link → Registration/login flow
- [ ] After registration: Coach associated with trainer, status "Pending" or "Active"
- [ ] Trainer can view invitation status (Pending, Accepted, Expired)
- [ ] Coach appears in trainer's Coaches list after acceptance
- [ ] Coach can ONLY be active under this trainer (no multi-trainer for coaches)

**Validation**:
- [ ] Email required
- [ ] If coach already exists: Cannot be active under different trainer (error message)
- [ ] Link expires: Clear message, option to resend invitation

---

### US-01.09: Player/Parent Sets Availability

**As a** Player or Parent  
**I want to** set my/my child's availability preferences  
**So that** trainers can schedule events when I'm available to attend

**Acceptance Criteria**:
- [ ] From navigation, click "Availability" or "My Times"
- [ ] Grid/calendar view: Days of week (rows/columns), time slots (hourly blocks or custom ranges)
- [ ] For each day: Toggle "Available" / "Not Available" OR select time ranges
- [ ] Example: "Monday: 5:00 PM - 8:00 PM", "Wednesday: Not Available"
- [ ] Save button → Availability stored per player profile
- [ ] If parent: Can set separate availability per child (profile switcher)
- [ ] Confirmation message: "Availability saved. Trainers can see these preferences when planning sessions."

**Trainer View**:
- [ ] In event creation or CRM: See player availability indicator
- [ ] Filter: "Show players available on [selected day/time]"
- [ ] Player card shows: "Best Times: Mon 5-8pm, Wed 6-9pm" (summary)
- [ ] Use for planning: Suggest session times matching most players' availability

---

### US-01.10: Coach Sets My Times (Availability)

**As a** Coach  
**I want to** define my weekly availability  
**So that** trainers schedule me for sessions I can attend

**Acceptance Criteria**:
- [ ] From Coach dashboard, click "My Times" or "Availability"
- [ ] Recurring schedule view: Select weekdays and time ranges
- [ ] Example: "Monday: 4:00 PM - 8:00 PM", "Saturday: 9:00 AM - 12:00 PM"
- [ ] Add multiple slots per day (e.g., "Monday: 4-6pm AND 7-9pm")
- [ ] Save → Coach availability stored in system

**Trainer Assignment Flow**:
- [ ] Trainer assigns coach to event at conflicting time
- [ ] System shows warning: "Coach [Name] is not available at this time per their schedule. Continue anyway?"
- [ ] Trainer can override with reason (text field required)
- [ ] Override logged: event_id, coach_id, override_reason, overridden_by (trainer_id)
- [ ] Coach sees assignment (no blocking), can accept or request change

---

### US-01.11: User Edits Own Profile

**As any** User  
**I want to** update my profile information  
**So that** my details are current and accurate

**Acceptance Criteria (All Roles)**:
- [ ] From navigation, click "Profile" or "Account Settings"
- [ ] Editable fields (common):
  - Name (first, last)
  - Phone number
  - Profile photo (upload or URL)
  - Optional: School, Bio (coaches), Jersey number (players)
- [ ] Read-only fields:
  - Email (login, cannot change - requires separate flow)
  - Role
  - Skill level (for players - trainer sets this)
  - Account created date
- [ ] Save → Changes persisted, confirmation message
- [ ] Profile photo: Upload to file storage, generate thumbnail, update photo URL
- [ ] Validation: Phone format, required fields enforced

**Role-Specific Fields**:
- **Player**: School, jersey number, photo
- **Parent**: Emergency contact info (if children)
- **Coach**: Bio, credentials, certifications, public profile checkbox
- **Trainer**: Business name, organization details
- **Super Admin**: Admin-specific settings (email notifications, etc.)

---

### US-01.12: Super Admin Deactivates User

**As a** Super Admin  
**I want to** deactivate a user account  
**So that** they cannot log in but their history is preserved

**Acceptance Criteria**:
- [ ] From Users tool, click "Deactivate" on user row
- [ ] Confirmation modal: "User will not be able to log in. All historical data will be preserved for analytics and compliance. Continue?"
- [ ] After confirm: User status → "Inactive"
- [ ] User cannot log in (error: "Account deactivated. Contact support.")
- [ ] User still appears in:
  - Historical analytics (attendance, payments, referrals)
  - Past event rosters
  - CRM records (grayed out or marked "Inactive")
- [ ] Super Admin can reactivate: Click "Reactivate" → Status "Active", user can log in again

---

### US-01.13: Super Admin Deletes User (GDPR Compliance)

**As a** Super Admin  
**I want to** permanently delete a user's personal information  
**So that** we comply with GDPR/privacy requests

**Acceptance Criteria**:
- [ ] From Users tool, click "Delete" on user row
- [ ] Confirmation modal (WARNING): "Personal information will be removed. Historical records will show 'Deleted User'. This cannot be undone. Continue?"
- [ ] After confirm: User fields anonymized:
  - Name → "Deleted User"
  - Email → "deleted_[user_id]@example.com"
  - Phone → NULL
  - Photo → Default avatar
  - Personal identifiers → NULL
- [ ] Historical records remain:
  - Event attendance: "Deleted User" attended session on [date]
  - Payments: "Deleted User" paid $X on [date]
  - Analytics totals unchanged (player count, revenue, attendance rates)
- [ ] User status → "Deleted"
- [ ] User cannot be reactivated (anonymization permanent)
- [ ] Deletion logged: Original user ID, who deleted, when, reason (for legal compliance)

---

### US-01.14: Trainer Customizes Portal Branding

**As a** Trainer  
**I want to** customize my portal's appearance  
**So that** my brand identity is reflected to players and coaches

**Acceptance Criteria**:
- [ ] Navigate to "My Portal Settings" or "Branding"
- [ ] **Logo Upload**:
  - Click "Upload Logo"
  - Select image file (PNG, JPG, max 2MB)
  - Logo displayed in header of trainer's portal
  - Logo visible to: Trainer's players, coaches, parents
  - Preview before save
- [ ] **Color Selection**:
  - Color picker for primary brand color
  - Used for UI gradient and accent colors
  - Preview changes in real-time
  - Reset to default option
- [ ] Save changes → Branding applied
- [ ] Changes visible immediately to all users in trainer's organization

**Validation**:
- [ ] Logo file type: PNG, JPG, SVG
- [ ] Logo file size: Max 2MB
- [ ] Logo dimensions: Recommended 200x200px, auto-resize if larger
- [ ] Color: Hex code format

**Scope for MVP**:
- ✅ Logo upload (single file)
- ✅ Primary color selection (for gradient)
- ❌ Multiple logos (light/dark mode) - Phase 2
- ❌ Font customization - Phase 2
- ❌ Full layout customization - Phase 2

**Use Cases**:
- Trainer "Elite Basketball Academy" uploads their logo and sets brand color to match their website
- Players see "Elite Basketball Academy" branding when they log in
- Coaches see consistent branding across the platform

---

## 8. Data Requirements

### What Information Needs to Be Stored

**For Users (All Roles)**:
- Email (unique, used for login)
- Password (securely hashed)
- Role (Super Admin, Trainer, Coach, Player/Parent)
- Status (Active, Inactive, Deleted)
- Email verification status
- Password reset tokens (temporary)
- Last login timestamp
- Account creation/update timestamps

**For Profiles (Common)**:
- First and last name
- Phone number
- Profile photo
- School/organization (optional)

**For Trainer Profiles**:
- Business name
- Organization details (address, website, description)
- Stripe integration IDs (for payments - Epic-05)
- Subscription status (for payments - Epic-05)
- Platform fee percentage (for payments - Epic-05)

**For Coach Profiles**:
- Which trainer they work for (ONE trainer only)
- Bio and credentials
- Certifications
- Public profile visibility setting
- When they joined the trainer

**For Player Profiles**:
- Player name (may differ from account if child)
- Age or birth date
- Gender
- Skill level (set by trainer)
- School, jersey number (optional)
- Is this a child profile?
- If child: Link to parent account
- Emergency contact info

**For Trainer-Player Associations** (Multi-Trainer Support):
- Which trainer
- Which player
- How they connected (via which ShareLink)
- When they connected
- Status (active/inactive)

**For ShareLinks** (Invitation System):
- Unique code (URL-safe)
- Type (static for players, unique for coaches)
- Which trainer owns it
- Who created it
- For coaches: Target email
- Expiration date (if applicable)
- Maximum uses (if applicable)
- How many times used
- Active status

**For Best Times / Availability**:
- For whom (coach or player)
- Day of week
- Start and end time
- Available or not available
- Created/updated timestamps

**For Impersonation Audit Log**:
- Which admin did the impersonation
- Which user was impersonated
- Start and end timestamps
- Duration
- Actions taken (optional detailed log)

**For Child Purchase Approvals**:
- Which child player
- Which parent needs to approve
- Which event/purchase
- Amount
- Status (pending, approved, denied, expired)
- Request and response timestamps
- Expiration timestamp (48 hours)
- Parent notes

**For Coach Availability Overrides**:
- Which event
- Which coach
- Which trainer overrode
- Reason for override (required)
- Timestamp

**For User Deletion Compliance**:
- Original user ID
- Original email (for legal compliance)
- Who deleted the user
- Reason for deletion
- When deleted
- Backup of original data (for legal compliance)

---

## 9. Business Rules & Logic

### Key Business Rules

**Authentication & Security**:
- Passwords must be securely hashed (industry-standard approach)
- Email must be unique across all users
- Sessions should expire after reasonable inactivity
- Password reset links expire after 1 hour
- Email verification links expire after 24 hours
- Login attempts should be rate-limited (prevent brute force)

**Role-Based Access**:
- Each user has exactly one role: Super Admin, Trainer, Coach, or Player/Parent
- After login, users see dashboard appropriate to their role
- Users can only access features permitted for their role
- Permissions enforced on both frontend (UI) and backend (API)

**Multi-Tenancy & Data Isolation**:
- Trainers can only see/manage their own organization's data
- Players can connect to multiple trainers (multi-trainer support)
- **Multi-trainer players see separated views**: Switch between trainer contexts, each with isolated data
- Coaches can only work for ONE trainer at a time (strictly enforced)
- When player connects to new trainer: No duplicate account, just new association

**Trainer Creation**:
- ONLY Super Admin can create trainer accounts (no self-registration)
- Ensures payment verification and quality control
- Trainer receives invitation email with setup instructions

**Player Registration**:
- Players register via ShareLink from trainer
- Static link for mass invitations (unlimited uses, no expiry)
- If player already has account: Auto-associate with new trainer
- If new: Create account and associate

**Coach Invitation**:
- Trainers invite coaches via unique ShareLink (one-time use, 7-day expiry)
- Coach cannot be active under multiple trainers simultaneously
- System validates coach isn't already active elsewhere

**Parent/Child Relationships**:
- Parent can create multiple child profiles
- **ALL players under 18 require parent-managed accounts** (no independent accounts for minors)
- Parent owns all contact information for family
- Each child has separate training calendar, RSVP status, Best Times

**Child Purchase Approval Workflow**:
- **USD Payments**: ALWAYS require parent approval
  - Child requests purchase → Status: "Pending Parent Approval"
  - Parent receives notification (email + in-app)
  - Parent can: Approve (payment processes), Deny (child notified)
  - Requests expire after 48 hours if no action
- **Token Spending**: Optional parental control setting
  - **Default**: Token spending requires parent approval (same workflow as USD)
  - **Optional**: Parent can enable "Allow child to spend tokens without approval"
  - Setting is per-child (parents can set different rules for each child)
- Parent can add notes when approving/denying any request

**Impersonation Rules**:
- Super Admin can impersonate any user EXCEPT other Super Admins
- Clear visual indicator when in impersonation mode
- All impersonation sessions logged (who, whom, start, end, duration)
- Impersonation session expires after 1 hour
- Used for support and troubleshooting

**User Deactivation (Soft Delete)**:
- User cannot log in
- ALL history preserved (analytics, attendance, payments, referrals)
- User still appears in historical records
- Can be reactivated by Super Admin

**User Deletion (GDPR Compliance)**:
- Personal information removed (name, email, phone)
- Historical records preserved but anonymized as "Deleted User"
- Analytics totals remain accurate (player counts, revenue, attendance rates)
- Deletion is permanent (cannot be reversed)
- Deletion logged for compliance

**Best Times / Availability**:
- Players/Parents set preferred training times (per player)
- Coaches set weekly availability (recurring schedule)
- Trainers can VIEW player availability (for planning)
- Trainers can FILTER events/players by availability match
- Used for scheduling suggestions, not restrictions

**Coach Availability Conflicts**:
- When trainer assigns coach to conflicting time: System shows warning
- Trainer can override with required reason (text explanation)
- Override is logged (who, when, why)
- Coach sees assignment (not blocked), can accept or request change

**ShareLink Tracking**:
- Track which ShareLink was used for each registration
- Track usage count per link
- Track when links are used (for analytics in Epic-06)
- Static links: Unlimited uses, no expiry
- Unique coach links: One use, 7-day expiry

**Validation Rules**:
- Email format validation
- Phone number format validation
- Required fields enforced (name, email for all users)
- Age validation for children (1-18 years)
- Duplicate email prevention
- Unique ShareLink codes

---

## 10. User Flows

### Flow 1: Player Registration via ShareLink

1. Player clicks trainer's ShareLink: `https://app.platform.com/join/ABC123`
2. If not logged in: Redirect to registration page
3. Registration form: Name, email, password, phone, player details (age, gender)
4. Submit registration
5. System creates account + player profile
6. System associates player with trainer who sent link
7. Confirmation email sent
8. Player logs in, sees trainer's events and content

**If player already has account**:
1. Player logs in
2. Clicks different trainer's ShareLink
3. System creates new trainer-player association (no duplicate account)
4. Player now sees events from both trainers

### Flow 2: Parent Creates Child Profile

1. Parent logs in to Player/Parent dashboard
2. Navigate to "Player Profiles"
3. Click "+ Add Player"
4. Enter child details: Name, age, gender, optional (school, photo)
5. Mark as "Child" (vs "Self")
6. Save
7. System links child profile to parent account
8. Parent can now switch between children using profile selector
9. Each child has separate: Training calendar, RSVP, Best Times

### Flow 3: Child Purchase Requires Approval

1. Child (logged in as child) browses events
2. Selects event requiring payment
3. Proceeds to checkout
4. System detects child account → Status: "Pending Parent Approval"
5. Parent receives notification (email + in-app alert)
6. Parent views pending request in dashboard
7. Parent reviews: Event details, cost, date/time
8. Parent approves → Payment processes, child registered for event
9. OR Parent denies → Child notified, no payment
10. Request expires after 48 hours if no action

### Flow 4: Super Admin Impersonates User

1. Super Admin logs in
2. Navigate to "Users" tool
3. Search/filter to find user
4. Click "Impersonate" button on user row
5. Confirmation: "View platform as [User Name]?"
6. Confirm
7. Portal switches to impersonated user's view
8. Top banner visible: "Viewing as [User Name] | Exit Impersonation"
9. Super Admin can now see exactly what user sees
10. Click "Exit Impersonation"
11. Return to Super Admin view
12. Session logged (start/end time, duration)

### Flow 5: Trainer Invites Coach

1. Trainer logs in
2. Navigate to "Coaches" section
3. Click "Invite Coach"
4. Enter coach email, optional message
5. System generates unique ShareLink (one-time use, 7-day expiry)
6. Invitation email sent to coach
7. Coach clicks link, registers/logs in
8. Coach associated with trainer
9. Coach appears in trainer's Coaches list
10. Coach can now be assigned to events

### Flow 6: Player Sets Best Times

1. Player/Parent logs in
2. Navigate to "Best Times" or "Availability"
3. Grid view: Days of week, time slots
4. For each day: Select available time ranges OR mark "Not Available"
5. Example: "Monday: 5:00 PM - 8:00 PM", "Wednesday: Not Available"
6. If parent: Select child from profile switcher, set separate availability
7. Save
8. Confirmation: "Availability saved. Trainers can see these preferences when planning."

**Trainer view**:
- In event creation: See "Players available at this time: 15 out of 20"
- In CRM: Filter "Show players available Monday 5-8pm"
- Player cards show: "Best Times: Mon 5-8pm, Wed 6-9pm"

### Flow 7: User Deletion (GDPR)

1. Super Admin receives deletion request from user
2. Navigate to "Users" tool
3. Find user, click "Delete"
4. Warning: "Personal information will be removed. Historical records will show 'Deleted User'. This cannot be undone."
5. Confirm deletion
6. System anonymizes: Name → "Deleted User", Email → deleted_[ID]@example.com, Phone → removed
7. Historical records preserved: Event attendance still shows "Deleted User attended", payments show "Deleted User paid $X"
8. Analytics totals unchanged (player counts, revenue sums remain accurate)
9. Deletion logged: Who deleted, when, reason
10. User status → "Deleted" (cannot be reversed)

---

## 11. Performance & Scale Targets

**Response Times**:
- Dashboard load: <2 seconds
- User list with 10,000 users: <3 seconds (with pagination)
- Profile save: <1 second
- ShareLink registration: <2 seconds

**Note**: Specific technical implementation (caching, indexing, etc.) decided by developer.

---

## 12. Questions / Open Issues

| ID | Question | Priority | Status | Owner |
|:---|:---|:---:|:---|:---|
| Q-01.01 | What are the specific skill level definitions? (Beginner, Intermediate, Advanced, Elite, or custom?) | P2 | Open | Client |
| Q-01.02 | How are age groups defined? (Birth year, age ranges, grade levels?) | P2 | Open | Client |
| Q-01.04 | What automated emails are required? (Welcome, password reset, invite, others?) | P1 | Open | Client |
| Q-01.05 | Email verification: Required before login or optional? | P1 | Open | Client |
| Q-01.06 | Coach availability override: Should coach be notified when overridden? | P2 | Open | Client |
| Q-01.07 | Session timeout: How long should users stay logged in? (1 day, 7 days, 30 days?) | P2 | Open | Client |

*Reference*: `05_Discussions/Open_Questions.md` for full question log

---

## 10. Acceptance Criteria (Epic-Level)

This Epic is complete when:

**Authentication & Authorization**:
- [ ] All 4 roles can log in with email/password
- [ ] Password reset flow works end-to-end
- [ ] Email verification sends and processes correctly
- [ ] Role-based access control enforced (correct dashboard per role)
- [ ] Session management works (login, logout, session expiry)
- [ ] Users cannot access features outside their permissions

**User Management (Super Admin)**:
- [ ] Super Admin can create trainer accounts
- [ ] Super Admin can edit any user account
- [ ] Super Admin can deactivate users (soft delete, history preserved)
- [ ] Super Admin can delete users (PII anonymized, "Deleted User" in history)
- [ ] Super Admin can impersonate any user (except other Super Admins)
- [ ] Impersonation logged with audit trail
- [ ] Users tool shows all users with tool-specific search and filters

**Player/Parent Workflows**:
- [ ] Player can register via ShareLink and associate with trainer
- [ ] Player can associate with multiple trainers (multi-trainer support)
- [ ] Parent can create child profiles
- [ ] Child purchase requires parent approval workflow complete
- [ ] Player/Parent can set Best Times availability
- [ ] Player profile management works (edit fields, upload photo)

**Coach Workflows**:
- [ ] Coach can be invited by trainer via unique ShareLink
- [ ] Coach can register and associate with trainer
- [ ] Coach can set My Times (weekly availability)
- [ ] Trainer sees availability conflict warning when assigning coach
- [ ] Trainer can override conflict with logged reason
- [ ] Coach can edit own profile (bio, credentials)

**Trainer Workflows**:
- [ ] Trainer can generate ShareLinks (static for players, unique for coaches)
- [ ] Trainer can view player Best Times in planning tools
- [ ] Trainer can assign coaches to events (with availability checks)
- [ ] Trainer can see own organization's players and coaches only

**Data Integrity**:
- [ ] All user data stored securely (passwords hashed, emails unique)
- [ ] Multi-tenancy enforced (trainers cannot see other trainers' data)
- [ ] Soft delete preserves history for analytics
- [ ] Delete anonymizes PII for GDPR compliance
- [ ] Audit logs capture all sensitive operations (impersonation, deletion)

**Performance**:
- [ ] Dashboard loads in <2 seconds
- [ ] User list (10,000 users) loads in <3 seconds with pagination
- [ ] Profile edits save in <1 second
- [ ] Platform handles 1,000 concurrent users

**Approval**:
- [ ] Demo approved
- [ ] All P0 questions resolved
- [ ] Security review passed (if applicable)

---

## 11. Mockups / Design References

**Key Screens to Design**:
1. Login page
2. Registration page (player, with parent/child option)
3. Super Admin: Users list
4. Super Admin: User creation modal
5. Super Admin: Impersonation banner
6. Player/Parent: Player Profiles list
7. Player/Parent: Player Profile edit
8. Player/Parent: Best Times availability grid
9. Coach: My Times availability grid
10. Trainer: ShareLink generation modal
11. Parent: Pending approvals list
12. User: Profile edit (all roles)

---

## 12. Testing Considerations

**What Should Be Tested**:

**Functional Testing**:
- All user stories and acceptance criteria met
- All user flows work end-to-end
- All business rules enforced correctly
- All validation rules working
- Error messages clear and helpful

**Key Scenarios to Test**:
- Player registers via ShareLink (new account and existing account)
- Multi-trainer player association (player connects to 2+ trainers)
- Parent creates child profile, child requests purchase, parent approves
- Super Admin creates trainer, impersonates user, exits impersonation
- Trainer invites coach (unique link, expiry, single-use)
- Coach sets availability, trainer overrides with reason
- User deactivation preserves history
- User deletion anonymizes PII correctly
- Best Times availability set by player, viewed by trainer
- ShareLink expiry and usage limits enforced

**Security Testing**:
- Users cannot access features outside their role
- Trainers cannot see other trainers' data
- Coaches cannot work for multiple trainers simultaneously
- Super Admin cannot impersonate other Super Admins
- Impersonation logged for audit
- Deleted user data properly anonymized
- Password security (hashing, complexity, reset flow)
- Rate limiting on login (prevent brute force)
- Session security (timeout, secure tokens)

**Performance Testing**:
- Dashboard loads in <2 seconds
- User list (10,000 users) loads in <3 seconds with pagination
- ShareLink registration handles 100 concurrent users
- Best Times queries fast with thousands of players

**Note**: Developer chooses specific testing tools and frameworks (unit, integration, E2E).

---

## 13. Implementation Notes

**Suggested Implementation Order**:
1. Core authentication and authorization
2. User management basics (CRUD, profiles)
3. ShareLink invitation system
4. Player/Parent features (profiles, children, Best Times)
5. Coach features (My Times, availability)
6. Super Admin tools (impersonation, audit logs)
7. Testing and refinement

**Security Requirements** (High-Level):
- Passwords must be securely hashed (industry-standard approach)
- Secure session management (prevent token theft, XSS attacks)
- CSRF protection on all state-changing operations
- Rate limiting on authentication endpoints (prevent brute force attacks)
- Token expiry: Email verification (24 hours), Password reset (1 hour), Impersonation sessions (1 hour)
- Audit logging for sensitive operations (impersonation, user deletion)

*Note: Specific security implementations decided by development team based on best practices*

**Accessibility Requirements**:
- WCAG 2.1 AA compliance
- Keyboard navigation for all forms
- Screen reader support
- Proper color contrast
- Clear focus indicators

**Mobile Requirements**:
- Responsive design (works on all screen sizes)
- Touch-friendly controls
- Mobile-optimized forms and uploads

---

**Priority**: P0 - Foundation (blocks all other epics)
**Complexity**: High (authentication, multi-role system, multi-tenancy)
**User Stories**: 12 (includes portal branding - US-01.12)

**Note**: This specification focuses on **business requirements** and **user needs**. Technical implementation details (API design, database schema, indexing strategies, specific technologies) are decided by the development team based on their expertise.

