## Overview

This document identifies the 8 major feature groups (Epics) for the platform MVP.

**Purpose**: Break down the system into manageable, logical feature groups that can be:
- Specified independently
- Developed in logical order
- Tested as cohesive units
- Prioritized for phased delivery

---

## Epic Dependency Map

```
                     Epic-01: User Management
                              │
                ┌─────────────┼─────────────┬────────────┐
                │             │             │            │
         Epic-02:        Epic-03:      Epic-04:     Epic-08: ⚡ NEW
       Event Mgmt      CRM & Players  LPPP Content  Forms & Registration
                │             │             │            │
                └──────┬──────┴─────────────┴────────────┘
                       │
                  Epic-05:
              Payments & Tokens
                       │
           ┌───────────┼───────────┐
           │           │           │
      Epic-06:    Epic-07:    Foundation:
    Marketing   Super Admin   Tech Stack
```

**Key Features**:
- **Epic-08**: Forms & Registration (Camps/Evaluations)
- Epic-08 depends on Epic-01 (user conversion) and Epic-05 (payments)
- Epic-07 integrates with Epic-08 (feature toggle)

---

## Epic 01: User Management & Authentication

### Overview
Multi-role authentication system with hierarchical permissions, supporting 7 distinct user types and complex relationships (multi-trainer players, parent-child accounts).

### Key Features
- User registration flows (trainer-created, share link, camp forms)
- Email/password authentication
- Role-based access control (RBAC)
- Parent-child account linking and permissions
- Multi-trainer player relationships
- Coach-trainer exclusive relationships
- Super Admin impersonation
- Password reset and account recovery
- User profile management per role

### Business Value
Foundation for all other features. Enables white-label multi-tenancy and complex permission requirements.

### MVP Scope
✅ **4 Core Roles**: Super Admin, Trainer, Coach, Player/Parent  
✅ Share link registration  
✅ Parent-child account management with approval workflows  
✅ Basic profile management per role  
✅ Impersonation for debugging  
✅ **Best Times / Availability** (players set preferences, trainers view)  
✅ **Portal Branding** (logo upload + color selection)  
❌ **Open Gym/League Instructor & Referee**: Confirmed OUT of MVP  

### Post-MVP
- Social login (Google, Facebook)
- Two-factor authentication (2FA)
- Advanced permission customization
- Single Sign-On (SSO) for enterprise
- Advanced portal branding (custom fonts, layouts, full white-label)

### Dependencies
- None (foundation epic)

### Estimated Complexity
**High** - Complex role hierarchy, parent-child relationships, multi-tenancy

### File
`Epic-01_User_Management_Authentication_SPEC.md`

---

## Epic 02: Event Management & Scheduling

### Overview
Creation, management, and RSVP system for training events (Sessions, Camps, Private sessions) with eligibility restrictions, capacity management, and coach assignments.

### Key Features
- Event Builder interface (create sessions, camps, privates)
- Event types with different behaviors
- Eligibility restrictions (age, level, gender, custom)
- Capacity management and waitlists
- RSVP system for players
- Coach assignment to events
- Recurring event creation
- Event calendar views (day, week, month)
- Bulk operations (Super Admin: clone, edit, cancel)
- Free event support (price = $0)
- Camp registration forms for non-logged-in users

### Business Value
Core business function. Enables trainers to manage their operations and players to book sessions.

### MVP Scope
✅ **3 Event Types** (Training Sessions, Camps, Privates)  
✅ Event Builder with eligibility restrictions  
✅ RSVP system with capacity management  
✅ Calendar views (day, week, month)  
✅ Coach assignment  
✅ Basic form creator for camps (template-based)  
✅ **Duplicate Event** feature (replaces recurring events for MVP)  
✅ **Private Events** (invitation-only, specific players/groups)  
✅ **Best Times integration** (view player availability when planning)  
✅ Bulk operations (simplified)  
❌ **NO recurring event series** (deferred to Phase 2)  
❌ **Open Gym events & League games**: Confirmed OUT of MVP  

### Post-MVP
- Recurring event series with proper business rules
- Event templates with auto-generation
- Waitlist automation
- Advanced form builder (drag-drop, custom fields)
- Weather cancellation automation
- iCal export for personal calendars
- Open Gym and League event types (if confirmed by client)

### Dependencies
- Epic-01: User Management (players to RSVP, coaches to assign)

### Estimated Complexity
**High** - Multiple event types, complex eligibility logic, form creation

### File
`Epic-02_Event_Management_Scheduling_SPEC.md`

---

## Epic 03: CRM & Player Management

### Overview
Comprehensive customer relationship management for trainers to track players, segment audiences, analyze engagement, and export data.

### Key Features
- Player profile creation and management
- Segmentation and filtering (age, level, attendance, custom tags)
- Tags and labels (custom organizational markers)
- Flags (medical, behavioral, financial, performance)
- Attendance history and tracking
- Session notes per player
- Search functionality
- Quick View analytics dashboard
- Top players (90-day attendance metric)
- No-show rate tracking
- Player communication logs
- Coach hours tracking (for trainer reference, payments handled externally)
- Event-scoped player access for coaches (see only players in assigned events)

### Business Value
Enables trainers to run their business effectively, make data-driven decisions, and provide personalized service.

### MVP Scope
✅ Player profiles with full data  
✅ Segmentation (age, level, attendance, custom tags)  
✅ Labels and flags system (medical, behavioral, financial, performance)  
✅ Attendance history tracking  
✅ **Tool-specific search** (search within CRM, not global)  
✅ **Quick View dashboard** (player trends, Top Players algorithm)  
✅ Core analytics (attendance, grade distribution, top players, no-show rates)  
✅ **ShareLink tracking** (see which link brought each player)  
✅ **Coach hours tracking** (total hours/sessions per coach for trainer reference)  
✅ **Event-scoped player access** (coaches see only players assigned to their events)  

### Post-MVP
- Advanced analytics (retention, churn prediction, lifetime value)
- Custom report builder
- Automated segmentation (ML-based)
- Player journey visualization
- Engagement scoring
- Batch operations on players

### Dependencies
- Epic-01: User Management (player accounts)
- Epic-02: Event Management (attendance data)

### Estimated Complexity
**Medium** - Data management, reporting, but straightforward CRUD operations

### File
`Epic-03_CRM_Player_Management_SPEC.md`

---

## Epic 05: Payment Processing & Financial Management

### Overview
Integrated payment system via Stripe supporting multiple payment models (tokens, subscriptions, per-event) and platform fee collection.

**Note**: Renumbered from Epic-04 to align with development sequence.

### Key Features
- Stripe integration (Checkout, Billing)
- Token system (pre-purchased credits)
- Subscription management (recurring monthly)
- Per-event payments
- Free event handling (price = $0)
- Payment method storage (cards on file)
- Transaction history
- Refund processing
- Platform fee collection (Dale's revenue - model TBD)
- Trainer payout tracking
- Invoice generation (manual if needed)
- Payment failure handling and retry

### Business Value
Enables monetization for both Dale (platform fees) and trainers (session/content revenue). Critical for business viability.

### MVP Scope
✅ **Stripe Connect (Express Accounts)** for trainer payouts  
✅ **Application Fee** model (5% platform cut included in price)  
✅ **Trainer-specific tokens** (pre-purchase and usage)  
✅ **Dual Pricing** (Events can have USD OR Token price, player chooses)  
✅ **Flexible Token Pricing** (Events can cost multiple tokens, not always 1:1)  
✅ **Trainer Can Gift Tokens** (Manual token additions for refunds, rewards, promotions)  
✅ **Player Subscriptions** (30-day unlimited access to qualifying events, special tokens activated from selected date)  
✅ **24-Hour Refund Policy** (Cancel 24+ hours before = full refund)  
✅ Per-event payments  
✅ Free events support ($0 price)  
✅ Payment method storage  
✅ Transaction history for users  
✅ **Per-trainer rates** (configurable platform cut per trainer)  
✅ **Monthly manual payouts** to trainers  
✅ Stripe Billing for trainer subscriptions  

### Post-MVP
- Trainer-to-coach payment in-app (Stripe Connect)
- Automated invoicing
- Financial analytics for trainers
- Multi-currency support
- Payment plans (installments)
- Scholarship/discount automation
- Token balance alerts (low balance notifications)
- Partial refunds (price change after RSVP)
- Subscription grace period (payment failure handling)

### Dependencies
- Epic-01: User Management (users to charge)
- Epic-02: Event Management (events to pay for)

### Estimated Complexity
**High** - Payment security, Stripe integration complexity, multiple payment flows

### File
`Epic-05_Payments_Tokens_SPEC.md`

---

## Epic 04: LP Content System (Learn/Practice)

### Overview
Playlist-driven training content delivery system allowing trainers to monetize educational content as upsells to players.

**Note**: Renumbered from Epic-05 to Epic-04 to reflect development priority. **Perfect Pillar removed from MVP January 22, 2026**.

### Key Features
- **Learn Pillar**: Tutorial video playlists (embedded YouTube/Vimeo) with text instructions
- **Practice Pillar**: Drill database with public/private toggle and text coaching points
- **Learn and Practice are functionally identical** (single implementation, different labels)
- **Text Content Support**: Trainers add setup instructions, key points, coaching notes to each video/drill
- Playlist creation and management
- Content organization (by skill, position, level, ability)
- Public/private content toggle
- **Coach-Only Content** (playlists visible only to coaches for certifications/training)
- **Coach Permissions**: Only trainers can create content (coaches can view only)
- Trainer Drill Database
- Content purchase flow (one-time or subscription)
- Player content library (purchased content)
- **Auto-complete progress tracking** (content marked complete when player starts watching)
- Super Admin public content library
- Trainer content ownership and sharing

### Business Value
Additional revenue stream for trainers. Differentiates platform from simple scheduling tools. Extends trainer value beyond in-person sessions.

### MVP Scope
✅ **Learn pillar** (YouTube embedded videos, unlisted) with **text instructions**  
✅ **Practice pillar** (drill database with public/private toggle) with **text coaching points**  
✅ Playlist creation and organization (by skill, position, level)  
✅ **Public content sharing** (trainers can mark content as discoverable)  
✅ Public/private toggle for all content types  
✅ **Coach-Only Content** (playlists restricted to coaches for training/certifications)  
✅ **Coach permissions**: Only trainers can create content (coaches view only)  
✅ Content purchase integration with payments (one-time or subscription)  
✅ **Trainer Drill Database** (searchable, organized library)  
✅ **Learn+Practice consolidation** (single backend implementation, different UI labels)  
✅ **Auto-complete progress** (marked complete when player starts watching)  
✅ **Progress preservation** (kept when playlist reassigned)  
❌ **Perfect pillar** (feedback layer) - **REMOVED FROM MVP (Jan 22, 2026)**  
❌ **Play pillar** (competitions, leaderboards) - Phase 2  

### Post-MVP
- **Perfect pillar** (Feedback Layer): Player provides video link + pays for text feedback from trainer
- Play pillar (competitions, leaderboards)
- Direct video uploads to platform (instead of external YouTube links)
- Video feedback from trainers
- AI-powered feedback
- Advanced progress tracking (watch time percentage, quiz/assessments)
- Manual "Mark as Complete" option
- Content marketplace (trainers sell to each other)
- Mobile app for better video experience

### Dependencies
- Epic-01: User Management (content creators and consumers)
- Epic-05: Payment Processing (content purchases)

### Estimated Complexity
**Medium-High** - Content management, video handling, feedback workflows

### File
`Epic-04_LPPP_Content_System_SPEC.md`

---

## Epic 06: Marketing & Growth Tools

### Overview
Tools for trainers to grow their business through referrals, promotions, and campaigns.

### Key Features
- "Get the Assist" referral program
- Referral code generation and tracking
- Referral rewards system
- Coupon/discount code creation
- Promotional campaigns (Black Friday, Christmas, etc.)
- Bulk messaging/email export for newsletters
- Referral analytics (conversion tracking)
- Campaign performance metrics
- Affiliate dashboard for trainers (if they refer other trainers)

### Business Value
Helps trainers grow player base. Network effects benefit Dale (more players = more transactions = more platform revenue).

### MVP Scope
✅ Referral program ("Get the Assist") - **Reward-based model**  
✅ Referral link generation (automatic per player)  
✅ Referral tracking (conversion analytics)  
✅ Platform-wide referral ratio (trainer views stats)  
✅ Coupon code creation (percentage or fixed amount)  
✅ Basic reward system (token rewards on first purchase)  
✅ Referral dashboard (trainer-facing analytics)  
❌ Email list export - **Post-MVP**  

### Post-MVP
- Advanced campaign management (A/B testing)
- Automated referral rewards
- Email campaign creation within platform
- SMS campaigns
- Social media integration
- Affiliate marketplace

### Dependencies
- Epic-01: User Management (referrers and referees)
- Epic-05: Payment Processing (reward redemption, discounts)

### Estimated Complexity
**Medium** - Tracking logic, discount application, but manageable scope

### File
`Epic-06_Marketing_Growth_Tools_SPEC.md`

---

## Epic 07: Super Admin & System Management

### Overview
Comprehensive admin tools for Dale to manage the entire platform, customize per-trainer settings, and monitor system health.

### Key Features
- User Role Editor (modify permissions)
- Test View (impersonate any user)
- System Customizer (global branding, feature toggles)
- Storage Monitor (cloud usage tracking)
- Event Master Tool (bulk event operations)
- Payout Manager (affiliate and coach payout tracking)
- Marketing Admin (control all campaigns)
- Revenue & Analytics (system-wide reporting)
- Feature toggle per trainer
- Trainer account management (create, deactivate, billing)
- Audit logs (track Super Admin actions)
- System health dashboard
- White-label customization per trainer

### Business Value
Enables Dale to operate platform efficiently, support trainers, debug issues, and manage business operations.

### MVP Scope
✅ User impersonation (Test View)  
✅ Trainer account creation and management  
✅ Feature toggles (limited set: LPPP, Marketing, Camps)  
✅ **Operational analytics** (users, events, engagement)  
✅ **Financial reporting via Stripe** (link to Stripe Dashboard, NO duplicate data)  
✅ Audit logging for critical actions  
✅ Pricing & Packages configuration  
✅ User Role Editor  
❌ **Bulk operations** - **Post-MVP**  
❌ **System health monitoring** - **Post-MVP**  
❌ **High-level financial charts on dashboard** - **Post-MVP** (Stripe is source of truth)  

**Note**: Portal branding (logo + color) is configured by trainers in Epic-01. Super Admin uses impersonation to help trainers with branding if needed.  

### Post-MVP
- Advanced analytics dashboard
- Automated billing management
- System health monitoring (uptime, performance)
- Bulk user operations
- Advanced feature toggle system
- Custom permission templates
- Trainer communication tools (in-app messaging)

### Dependencies
- Epic-01: User Management (users to manage)
- All Epics (Super Admin has access to all features)

### Estimated Complexity
**Medium-High** - Many features, but most are administrative CRUD with elevated permissions

### File
`Epic-07_Super_Admin_System_Management_SPEC.md`

---

## Epic 08: Forms & Registration (Camps/Evaluations)

### Overview
Forms-based registration system for camps and evaluations—distinct from calendar events. Enables external lead generation, payment collection, and user conversion.

### Key Features
- Form builder with pre-loaded templates (Camps, Evaluations)
- Form fields: Text, Email, Dropdown, Multi-select
- Shareable external links (social media, website, email)
- Capacity limits (camps only)
- Optional Stripe payment integration
- Turn on/off capability (camps temporary, evaluations permanent)
- Player conversion to full user accounts
- Trainer views form submissions and participant lists
- Export participant data

### Business Value
Lead generation and player acquisition. Converts camp participants into platform users. Monetizes camps and evaluations with Stripe payments.

### MVP Scope
✅ **Simple form builder** (template-based, add/remove/reorder fields)  
✅ **Camps** (temporary forms with capacity limits)  
✅ **Evaluations** (always-on forms)  
✅ **Stripe payment integration** (optional per form)  
✅ **Camp-to-user conversion flow** (seamless account creation)  
✅ **Participant management** (view submissions, mark attendance, export)  
✅ **Feature toggle** (Super Admin can enable/disable per trainer)

### Post-MVP
- Complex form fields (file uploads, signatures)
- Conditional logic (show/hide fields based on answers)
- Multi-page forms
- Form analytics (completion rates, drop-off)
- Evaluation time slot selection
- Group discounts and early bird pricing
- Automated email reminders

### Dependencies
- **Requires**: Epic-01 (user conversion), Epic-05 (Stripe payments)
- **Integrates With**: Epic-07 (feature toggle)

### Estimated Complexity
**Medium-High** - Form builder + Stripe integration + conversion flow

### File
`Epic-08_Forms_Registration_SPEC.md`

**Key Decisions**:
- Camps & Evaluations as forms (not calendar events)
- Camps IN MVP as separate Epic-08
- Feature toggle "Camps" added to Epic-07

---

## MVP Prioritization

### Phase 1A (Must Have - Core Flow)
1. **Epic-01**: User Management (foundation)
2. **Epic-02**: Event Management (core business function)
3. **Epic-05**: Payment Processing (monetization)

**Goal**: Trainers can create events, players can RSVP and pay. Basic CRM.

---

### Phase 1B (Must Have - Differentiation)
4. **Epic-03**: CRM & Player Management (business operations)
5. **Epic-06**: Marketing Tools (basic referrals, coupons)

**Goal**: Trainers can manage players and grow business.

---

### Phase 1C (Must Have - Upsell Value)
6. **Epic-04**: LPPP Content System (revenue enhancement)

**Goal**: Trainers can monetize content. Platform differentiated from competitors.

---

### Phase 1D (Must Have - Platform Management)
7. **Epic-07**: Super Admin Tools (operational necessity)

**Goal**: Dale can manage platform, onboard trainers, debug issues.

---

## Post-MVP Epics (Phase 2+)

### Epic 08: Advanced Communication (Phase 2)
- In-app messaging
- SMS notifications
- Push notifications (if mobile app)
- Video calls (if needed for remote coaching)

### Epic 09: Mobile Applications (Phase 2/3)
- iOS app
- Android app
- Offline mode
- Push notifications

### Epic 10: Advanced Analytics & Reporting (Phase 2)
- Predictive analytics (churn, retention)
- Custom report builder
- Benchmarking (compare to averages)
- Financial forecasting

### Epic 11: Marketplace & Network Effects (Phase 3)
- Trainer-to-trainer content sales
- Player cross-trainer discovery
- Tournament/event hosting across trainers
- Play pillar (LPPP) - competitions and leaderboards

---

## Epic Summary

| Epic | Stories | File |
|:-----|:-------:|:-----|
| Epic-01: User Management | 14 | `Epic-01_User_Management_Authentication_SPEC.md` |
| Epic-02: Event Management | 15 | `Epic-02_Event_Management_Scheduling_SPEC.md` |
| Epic-03: CRM & Players | 13 | `Epic-03_CRM_Player_Management_SPEC.md` |
| Epic-04: LP Content | 11 | `Epic-04_LPPP_Content_System_SPEC.md` |
| Epic-05: Payments | 10 | `Epic-05_Payments_Tokens_SPEC.md` |
| Epic-06: Marketing Tools | 8 | `Epic-06_Marketing_Growth_Tools_SPEC.md` |
| Epic-07: Super Admin | 9 | `Epic-07_Super_Admin_System_Management_SPEC.md` |
| Epic-08: Forms & Registration | 6 | `Epic-08_Forms_Registration_SPEC.md` |
| **TOTAL** | **86** | **ALL EPICS** |

---

## Recommended Development Order

**Recommended Development Order**: Epic-01 → Epic-05 → Epic-02 → Epic-03 → Epic-04 → Epic-06 → Epic-07 → Epic-08

---

## Related Files

- [Client Brief](../01_Context/Client_Brief.md) - Requirements source
- [System Overview](../01_Context/System_Overview.md) - Architecture context
- [Epic Template](../00_Meta/Templates/Epic_Template.md) - Use for detailed specs
- [Feature Template](../00_Meta/Templates/Feature_Template.md) - Child feature specs
- [Project Plan](../00_Meta/Project_Plan.md) - Timeline and milestones

---

## Cross-Epic References & Integration Points

This section documents where Epics reference each other's features, helping developers understand dependencies and integration requirements.

### Epic-01 → Used By:
- **Epic-02**: Uses user authentication, player availability (Best Times), parent approval workflows, coach assignments
- **Epic-03**: Uses player accounts, tags, notes, medical flags from profiles
- **Epic-04**: Uses user accounts (trainers create content, players consume)
- **Epic-05**: Uses parent accounts for payment methods, Stripe Customer association
- **Epic-06**: Uses user accounts for referrals (referrer/referee)
- **Epic-07**: Uses all user roles for management

**Key Integration Points**:
- User authentication (all Epics need logged-in users)
- Player availability data (US-01.05 → Epic-02 scheduling)
- Parent approval workflow (US-01.04 → Epic-02 RSVPs, Epic-05 payments)
- Coach assignments (US-01.09 → Epic-02 sessions)

---

### Epic-02 → Used By:
- **Epic-03**: Attendance data (US-02.11) feeds into CRM analytics
- **Epic-05**: Event payments require event data, RSVP triggers payment flow
- **Epic-06**: ShareLinks can link to specific events (marketing)

**Key Integration Points**:
- Event RSVP triggers payment (US-02.07 → Epic-05)
- Attendance tracking (US-02.11 → Epic-03 analytics)
- Event cancellation triggers refunds (US-02.13 → Epic-05)

---

### Epic-03 → Used By:
- **Epic-02**: Player roster used for event eligibility, private event invitations
- **Epic-06**: Bulk messaging uses player segmentation

**Key Integration Points**:
- Player roster (US-03.01 → Epic-02 event invitations)
- Player tags (US-03.04 → Epic-02 eligibility filtering)
- Attendance history (US-03.08 uses Epic-02 data)

---

### Epic-04 → Used By:
- **Epic-05**: Content purchases require payment processing

**Key Integration Points**:
- Content purchase (US-04.07 → Epic-05 payment flow)
- Public content library (US-04.09 → cross-trainer discovery)

---

### Epic-05 → Used By:
- **Epic-02**: Event payments, refunds
- **Epic-04**: Content access payments
- **Epic-06**: Token rewards, coupon discounts

**Key Integration Points**:
- Payment processing (US-05.02, US-05.03 → all paid features)
- Refunds (US-05.04 → Epic-02 cancellations)
- Token balance (US-05.01 → Epic-02 token payments)
- Platform fee collection (US-05.06 → Dale's revenue)

---

### Epic-06 → Used By:
- **Epic-05**: Referral rewards (token awards)

**Key Integration Points**:
- Referral tracking (US-06.01 → Epic-05 token awards)
- Coupon redemption (US-06.05 → Epic-05 payment discounts)

---

### Epic-07 → Uses All Epics:
- Manages trainers (Epic-01)
- Views all events (Epic-02)
- Configures features (Epic-04 LPPP, Epic-06 Marketing, Epic-08 Camps)
- Views operational metrics (all Epics)

**Key Integration Points**:
- Impersonation (US-07.03 → all Epics)
- Feature toggles (US-07.04 → Epic-04, Epic-06, Epic-08)
- Pricing configuration (US-07.05 → Epic-05)

---

### Epic-08 → Used By:
- **Epic-01**: User conversion flow (camp submissions → user accounts)
- **Epic-05**: Stripe payment for paid camps/evaluations
- **Epic-07**: Feature toggle configuration

**Key Integration Points**:
- Form submission (US-08.03 → Epic-05 Stripe Checkout)
- User conversion (US-08.05 → Epic-01 account creation)
- Feature toggle (US-07.04 → enable/disable Camps per trainer)

---

## Reducing Epic Interdependence (Strategies)

**Challenge**: Current dependencies create development constraints - can't build Epic-02 without Epic-01, can't test Epic-05 without Epic-02, etc.

**Strategies to Consider**:

### 1. Mock/Stub Approach (Recommended)
- Build Epic-02 with **mocked users** (hard-coded test accounts)
- Build Epic-05 with **mocked Stripe** (test mode responses)
- Test Epics independently before integration

**Pros**: Parallel development, independent testing  
**Cons**: Need integration phase at end

### 2. Decouple Where Possible
- **Epic-04 (LPPP)**: Could launch with free content initially (no Epic-05 dependency)
- **Epic-03 (CRM)**: Basic roster doesn't need Epic-02 attendance (defer analytics)
- **Epic-06 (Marketing)**: Referral tracking doesn't require rewards (Epic-05)

**Pros**: Faster MVP, incremental launches  
**Cons**: Some features incomplete

### 3. API-First Development
- Define API contracts between Epics upfront
- Each Epic builds to its API contract
- Integration is connecting pre-defined APIs

**Pros**: Clear boundaries, parallel work  
**Cons**: Requires upfront API design

### 4. Feature Flags for Dependencies
- Epic-02 works with/without Epic-05 (payments optional)
- Epic-03 works with/without Epic-02 (attendance optional)
- Toggle features on as Epics complete

**Pros**: Continuous delivery, flexible deployment  
**Cons**: More code complexity (conditional logic)

**Recommendation**: Combination of #1 (mocking for development) and #2 (strategic decoupling where it doesn't hurt MVP value).

---

**All Epics**: Epic-01 through Epic-08 (**86 user stories**)

