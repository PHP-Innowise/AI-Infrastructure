# Epic-06: Marketing & Growth Tools

---

## 1. Epic Overview

### Purpose
Provide trainers with tools to grow their player base through referrals and promotional campaigns. Enable word-of-mouth marketing and incentivize player acquisition through the "Get the Assist" referral program and discount coupon system.

### Business Value
- **For Trainers**: Organic growth through player referrals, reduced customer acquisition cost
- **For Players**: Rewards for referring friends, discounts for joining/attending
- **For Platform Owner**: Network effects (more players = more transactions = more revenue)

### Success Metrics
- ✅ 40%+ of new players come through referrals (within 3 months)
- ✅ Average 2+ referrals per active player per year
- ✅ 60%+ referral conversion rate (referral makes first purchase)
- ✅ 30%+ of trainers actively use coupon codes
- ✅ Average 10%+ revenue increase from promotional campaigns

---

## 2. Scope Summary

**Goals**:
- Reward-based referral system ("Get the Assist")
- Automatic referral link generation per player
- Platform-wide referral reward rules (simplified)
- Coupon/discount code creation and redemption
- Referral and coupon analytics

**Non-Goals** (Post-MVP):
- Email list export (Phase 2)
- Campaign management (grouping multiple coupons)
- Automated campaigns (e.g., "Welcome discount for all new players")
- Social media integration
- Affiliate marketplace (trainer-to-trainer referrals)
- SMS marketing
- A/B testing for campaigns

---

## 3. In Scope (MVP)

### Referral System ("Get the Assist")
- [ ] Automatic referral link per player (no generation needed)
- [ ] Referral tracking via ShareLink system
- [ ] **Platform-wide referral reward rules** (Super Admin configures)
- [ ] Token rewards on referral's first purchase
- [ ] Referral dashboard for trainers
- [ ] Referral stats per player

### Coupon System
- [ ] Create discount codes (percentage or fixed amount)
- [ ] Apply to events and/or content
- [ ] Expiration dates and usage limits
- [ ] Coupon redemption tracking
- [ ] Coupon analytics

---

## 4. Out of Scope (Post-MVP)

**Phase 2 Deferrals**:
- ❌ Email list export for newsletters
- ❌ Campaign management (Black Friday Sale with multiple coupons)
- ❌ Automated campaigns (trigger-based discounts)
- ❌ Per-trainer referral reward customization (platform-wide only for MVP)
- ❌ Alternative reward tracking (T-shirts, etc.) - trainers handle outside platform
- ❌ Social media sharing integration
- ❌ Referral contests/leaderboards
- ❌ SMS campaigns
- ❌ Trainer-to-trainer affiliate program

---

## 5. Dependencies

### Required Before This Epic
- **Epic-01** (User Management) - ShareLink system for referral tracking
- **Epic-05** (Payments) - Token system for rewards, payment processing for first purchase trigger

### Blocks These Epics
- None (can be developed in parallel with Epic-07)

### External Dependencies
- None (uses internal systems only)

---

## 6. User Roles Involved

| Role | Interaction | Permissions |
|:---|:---|:---|
| **Trainer / Business Owner** | Creates coupons, views referral analytics | Can create/edit own coupons, view own referral data |
| **Player / Parent** | Refers friends, uses coupons | Can share referral link, apply coupons to purchases |
| **Super Admin** | Configures platform-wide referral rules, views all coupon usage | Full access to all referral/coupon data, configures global rules |

---

## 7. User Stories & Acceptance Criteria

### US-06.01: Player Has Automatic Referral Link

**As a** Player  
**I want to** have a referral link I can share  
**So that** I can invite friends and earn rewards

**Acceptance Criteria**:
- [ ] Every player automatically has a referral link (no "generate" action needed)
- [ ] Link format: `platform.com/join/{trainer-slug}/{player-id}` or similar
- [ ] Player can view their referral link in profile or dashboard
- [ ] "Share" button copies link to clipboard
- [ ] Link is always active (no expiration)
- [ ] Link is unique per player-trainer relationship

**Multi-Trainer Context**:
- [ ] If player trains with multiple trainers, they have separate referral links per trainer
- [ ] Example: Sarah with Coach Bob has link for Bob, Sarah with Coach Lisa has different link for Lisa
- [ ] Referrals are credited to the specific trainer's program

**Display**:
- [ ] "Get the Assist" section in player dashboard
- [ ] Shows: Your referral link, copy button, referral count
- [ ] "You've referred 3 players (2 converted)" stats

---

### US-06.02: Player Refers Friend

**As a** Player  
**I want to** share my referral link with friends  
**So that** they can join my trainer's program

**Acceptance Criteria**:
- [ ] Player copies referral link from dashboard
- [ ] Shares via text, email, social media (outside platform)
- [ ] Friend clicks link
- [ ] Friend taken to registration page for that specific trainer
- [ ] System tracks: Referral link clicked, timestamp, referrer (Player A), referee (Player B)
- [ ] After registration: Referral recorded as "Pending" (awaiting first purchase)

**Attribution Rules**:
- [ ] **30-day attribution window** (Q-06.12 - Assumption)
  - Friend clicks link, can register within 30 days
  - After 30 days: Attribution expires
- [ ] **Last-click attribution**
  - If friend clicks multiple referral links, last one wins
- [ ] **Browser cookie tracking**
  - Stores referrer info in cookie for 30 days

**Edge Cases**:
- [ ] If friend already has account: Link still works, but no referral credit
- [ ] If friend registers with different trainer: No referral credit

---

### US-06.03: System Awards Referral Rewards (First Purchase Trigger)

**As a** Player  
**I want to** receive rewards when my referrals make their first purchase  
**So that** I'm incentivized to refer more friends

**Reward Trigger**:
- [ ] Referee (Player B) makes **FIRST PURCHASE** (event RSVP, content purchase, or token purchase)
- [ ] System detects: This is first purchase for Player B
- [ ] System looks up: Who referred Player B? (Player A)
- [ ] System checks: Are both Player A and Player B active?

**Platform-Wide Reward Rules** (Simplified):
- [ ] **Super Admin configures global rule**: "X referrals who purchase = 1 free token"
- [ ] Example: "3 referral purchases = 1 token"
- [ ] **Q-06.11** (To confirm with client): What should the ratio be?
  - Option A: 3:1 (3 referral purchases = 1 token)
  - Option B: 5:1 (5 referral purchases = 1 token)
  - Option C: 1:1 (1 referral purchase = 1 token) - very generous
- [ ] Rule applies to ALL trainers (no per-trainer customization for MVP)

**Token Grant**:
- [ ] After X referral purchases, Player A receives 1 token (trainer-specific)
- [ ] Token added to Player A's balance with that trainer
- [ ] Notification sent: "You earned 1 token! Your friend [Name] just joined."
- [ ] Referee (Player B) also receives token (optional - to confirm with client)

**Tracking**:
- [ ] System tracks "assist count" per player-trainer relationship
- [ ] Example: Player A has 7 assists with Coach Bob (7 referrals who purchased)
- [ ] If rule is 3:1, Player A has earned 2 tokens (with 1 assist toward next token)

**Anti-Abuse**:
- [ ] Reward triggers ONLY on first purchase (not every purchase)
- [ ] Cannot refer yourself
- [ ] Cannot create fake accounts (requires payment to trigger reward)

---

### US-06.04: Trainer Views Referral Dashboard

**As a** Trainer  
**I want to** see which players are referring others  
**So that** I can recognize and reward top advocates

**Acceptance Criteria**:
- [ ] Navigate to "Marketing" → "Referrals" or "Get the Assist"
- [ ] Dashboard shows:

**Overview Metrics**:
- [ ] Total referrals this month
- [ ] Total conversions this month (referrals who purchased)
- [ ] Conversion rate (%)
- [ ] Total referral revenue (sum of first purchases from referrals)

**Top Referrers** (leaderboard):
- [ ] Player name
- [ ] Total referrals sent
- [ ] Total conversions (referrals who purchased)
- [ ] Conversion rate (%)
- [ ] Last referral date

**Referral Activity Log**:
- [ ] Date
- [ ] Referrer (Player A)
- [ ] Referee (Player B)
- [ ] Status: Pending (not purchased yet), Converted (purchased), Rewarded (token granted)
- [ ] First purchase amount (if converted)

**Use Case**:
- Trainer sees: "Sarah has referred 10 players, 8 converted"
- Trainer can manually reward Sarah outside platform (e.g., free session, T-shirt)
- Platform just provides the data

---

### US-06.05: Trainer Creates Coupon Code

**As a** Trainer  
**I want to** create discount codes  
**So that** I can run promotions and attract new players

**Acceptance Criteria**:
- [ ] Navigate to "Marketing" → "Coupons"
- [ ] Click "Create Coupon"
- [ ] Fill form:
  - **Code**: Text field (e.g., "SUMMER20", "NEWPLAYER10")
  - **Discount Type**: Percentage (%) or Fixed Amount ($)
  - **Discount Value**: Number (e.g., 20 for 20% off, or 10 for $10 off)
  - **Applies To**: Events, Content, or Both (Q-06.10 - needs confirmation)
  - **Usage Limit**: Unlimited, Single-Use, or Multi-Use (e.g., 50 uses)
  - **Expiration Date**: Date picker (optional)
  - **Status**: Active or Inactive

**Validation**:
- [ ] Code must be unique (within trainer's coupons)
- [ ] Code: 4-20 characters, alphanumeric + hyphens
- [ ] Discount value: > 0
- [ ] Percentage discounts: 1-100%
- [ ] Fixed discounts: Cannot exceed event/content price (checked at redemption)
- [ ] Expiration date: Must be future date (if set)

**After Creation**:
- [ ] Coupon appears in trainer's coupon list
- [ ] Trainer can view, edit, deactivate coupon
- [ ] Trainer can share code with players (via social media, website, etc.)

**Use Cases**:
- "SUMMER20" - 20% off all events in June
- "FIRSTCLASS" - $10 off first event for new players
- "NEWPLAYER" - 50% off first month (for new registrations)

---

### US-06.06: Player Uses Coupon Code

**As a** Player or Parent  
**I want to** apply a coupon code to my purchase  
**So that** I can get a discount

**Acceptance Criteria**:
- [ ] At checkout (event RSVP or content purchase):
  - Payment screen shows: "Have a coupon code?"
  - Text input: "Enter code"
  - Button: "Apply"

**Apply Coupon Flow**:
- [ ] Player enters code (e.g., "SUMMER20")
- [ ] Click "Apply"
- [ ] System validates:
  - Code exists for this trainer
  - Code is active
  - Code has not expired
  - Usage limit not reached
  - Player eligible (Q-06.10 - new vs. existing players)
- [ ] If valid:
  - Discount applied in-platform
  - Price updated on screen
  - Example: "$20 → $16 (20% off with SUMMER20)"
  - Proceed to payment with final discounted price
  - Stripe receives $16 (final amount, no coupon details sent to Stripe)
- [ ] If invalid:
  - Error message: "Invalid or expired code"
  - No discount applied

**Eligibility Rules** (Q-06.10 - To confirm with client):
- **Option A**: Coupons for new players only (acquisition)
  - Only players with 0 purchases can use
  - Blocks existing players
- **Option B**: Coupons for all players (acquisition + retention)
  - Any player can use (default assumption)
- **Option C**: Trainer decides per coupon
  - Checkbox: "New players only"

**Coupon Stacking**:
- [ ] Only ONE coupon per transaction
- [ ] Cannot combine multiple codes

**Tracking**:
- [ ] Coupon usage logged: Which player, which event/content, discount amount, timestamp
- [ ] Usage count incremented
- [ ] If usage limit reached: Code auto-deactivated

---

### US-06.07: Trainer Views Coupon Analytics

**As a** Trainer  
**I want to** see coupon performance  
**So that** I can measure campaign effectiveness

**Acceptance Criteria**:
- [ ] Navigate to "Marketing" → "Coupons"
- [ ] View list of all coupons with stats:

**Per Coupon**:
- [ ] Code
- [ ] Discount (e.g., "20% off")
- [ ] Status (Active, Expired, Inactive)
- [ ] Total uses
- [ ] Usage limit (if set)
- [ ] Total discount given ($ amount)
- [ ] Revenue generated (sum of purchases using this coupon)
- [ ] Conversion rate (% of new players who used this coupon and completed purchase)
- [ ] Created date, expiration date

**Actions**:
- [ ] Edit coupon (change expiration, usage limit, status)
- [ ] Deactivate coupon (stop new uses)
- [ ] Delete coupon (if never used)
- [ ] View usage details (list of players who used it)

**Analytics Summary** (all coupons combined):
- [ ] Total coupons created
- [ ] Active coupons
- [ ] Total coupon uses this month
- [ ] Total discount given this month
- [ ] Revenue from coupon users this month

---

### US-06.08: Super Admin Configures Referral Reward Rules

**As a** Super Admin  
**I want to** set the platform-wide referral reward ratio  
**So that** the referral program is consistent across all trainers

**Acceptance Criteria**:
- [ ] Navigate to "System Settings" → "Referral Program"
- [ ] View current rule: "3 referral purchases = 1 token"
- [ ] Click "Edit"
- [ ] Modify:
  - **Referrals Required**: Number (e.g., 3, 5, 10)
  - **Tokens Awarded**: Number (default: 1)
  - **Reward Referee Too**: Checkbox (give token to new player as welcome bonus?)
- [ ] Save changes
- [ ] Rule applies to ALL trainers immediately
- [ ] Existing assist counts preserved (e.g., if player has 2 assists under old 3:1 rule, they keep 2 assists under new 5:1 rule)

**Audit Logging**:
- [ ] Log: Who changed rule, when, old value, new value

**Q-06.11** (To confirm with client):
- What should the default ratio be?
- Should referee also get a token (welcome bonus)?

---

## 8. Data Requirements

### What Information Needs to Be Stored

**For Referrals**:
- Referrer (Player A) account reference
- Referee (Player B) account reference
- Trainer reference
- Referral date (when link was clicked / registration completed)
- Attribution source (referral link ID)
- Status: Pending, Converted, Rewarded
- First purchase date (if converted)
- First purchase amount (if converted)

**For Assist Counts** (per player-trainer):
- Player reference
- Trainer reference
- Total assists (count of referrals who purchased)
- Tokens earned from referrals
- Last assist date

**For Coupons**:
- Coupon unique identifier
- Trainer reference
- Code (text, unique per trainer)
- Discount type (percentage or fixed)
- Discount value
- Applies to (events, content, both)
- Usage limit (optional)
- Current usage count
- Expiration date (optional)
- Status (active, inactive, expired)
- Created timestamp
- Created by (trainer)

**For Coupon Usage**:
- Usage unique identifier
- Coupon reference
- Player reference
- Transaction reference (event RSVP or content purchase)
- Discount amount applied
- Original price
- Final price after discount
- Usage timestamp

**For Referral Rewards**:
- Reward unique identifier
- Player reference (who received reward)
- Trainer reference
- Reward type (token)
- Reward amount (1 token)
- Trigger event (referral ID that triggered reward)
- Timestamp

---

## 9. Business Rules & Logic

### Referral Attribution Rules

**Attribution Window**:
- 30 days from link click to registration (Q-06.12 - Assumption)
- After 30 days: Link attribution expires

**Attribution Method**:
- Last-click attribution (most recent referral link wins)
- Stored in browser cookie for 30 days

**Edge Cases**:
- Player already registered: No referral credit
- Player clicks link but registers via different trainer: No referral credit
- Self-referral: Blocked (cannot refer yourself)

### Referral Reward Rules

**Platform-Wide Rule** (Simplified):
- Super Admin sets: "X referrals who purchase = Y tokens"
- Default (to confirm): 3 referrals = 1 token (Q-06.11)
- Rule applies to ALL trainers (no per-trainer customization for MVP)

**Reward Trigger**:
- Referee makes FIRST PURCHASE (event, content, or tokens)
- System increments referrer's assist count
- When assist count reaches threshold (e.g., 3), grant token
- Reset counter and repeat (player can earn unlimited tokens)

**Reward Delivery**:
- Token added to referrer's balance with that trainer
- Notification sent: "You earned 1 token! Your friend [Name] just joined."
- Optional: Referee also gets token as welcome bonus (to confirm with client)

**Multi-Trainer Context**:
- Assists tracked per player-trainer relationship
- Player A refers someone to Coach Bob → Assist counted for Player A with Coach Bob
- Player A refers someone to Coach Lisa → Separate assist count for Player A with Coach Lisa

### Coupon Rules

**Validation**:
- Code must be active
- Code must not be expired (if expiration date set)
- Usage limit not reached (if set)
- Player eligible (based on new vs. existing rules - Q-06.10)

**Discount Application**:
- Percentage discounts: Applied to original price
  - Example: $20 event, 20% off = $16
- Fixed discounts: Subtracted from original price
  - Example: $20 event, $5 off = $15
- Minimum price: $0 (cannot go negative)
  - Example: $5 event, $10 off = $0 (free)

**Coupon Stacking**:
- Only one coupon per transaction
- Cannot combine multiple codes

**Trainer Scope**:
- Coupons created by Trainer A only work for Trainer A's events/content
- Players cannot use Trainer A's coupon for Trainer B's events

---

## 10. User Flows

### Flow 1: Player Refers Friend and Earns Token

1. Player A (Sarah) logs in to Coach Bob's platform
2. Navigate to dashboard, sees "Get the Assist" section
3. Referral link shown: `platform.com/join/coach-bob/sarah-123`
4. Click "Copy Link" button
5. Share link with friend (Player B - Mike) via text message
6. Mike clicks link, taken to registration page for Coach Bob
7. Mike registers (becomes Player B)
8. System tracks: Sarah referred Mike to Coach Bob (assist pending)
9. Mike browses events, RSVPs to "Basketball Skills - $20"
10. Mike completes payment (first purchase)
11. System detects: Mike's first purchase
12. System increments Sarah's assist count: 2 → 3 assists
13. System checks platform rule: 3 assists = 1 token
14. System grants Sarah 1 token with Coach Bob
15. Sarah receives notification: "You earned 1 token! Mike just joined."
16. Sarah's assist count resets to 0 (starts earning toward next token)

### Flow 2: Trainer Creates Coupon and Player Uses It

1. Trainer logs in
2. Navigate to "Marketing" → "Coupons"
3. Click "Create Coupon"
4. Fill form:
   - Code: "NEWPLAYER"
   - Discount: 50% off
   - Applies to: Events
   - Usage limit: 100 uses
   - Expiration: June 30, 2026
5. Save coupon
6. Coupon appears in list: "NEWPLAYER - 50% off - 0 / 100 uses"
7. Trainer shares code on social media: "Join with code NEWPLAYER for 50% off!"

8. New player (Player C) registers with Coach Bob
9. Player C browses events, selects "Basketball Skills - $20"
10. At checkout, sees: "Have a coupon code?"
11. Enters: "NEWPLAYER"
12. Clicks "Apply"
13. System validates: Code exists, active, not expired, usage limit OK
14. Discount applied: $20 → $10 (50% off)
15. Screen updates: "Price: $10 (50% off with NEWPLAYER)"
16. Player C completes payment for $10
17. System logs: NEWPLAYER used by Player C, $10 discount
18. Coupon usage count: 0 → 1 uses

19. Trainer views coupon analytics
20. Sees: "NEWPLAYER - 1 use, $10 discount given, $10 revenue"

### Flow 3: Super Admin Adjusts Referral Reward Ratio

1. Super Admin logs in
2. Navigate to "System Settings" → "Referral Program"
3. Current rule: "3 referral purchases = 1 token"
4. Feedback: Too stingy, players not motivated
5. Click "Edit"
6. Change: "2 referral purchases = 1 token"
7. Save
8. System updates global rule
9. All trainers now use new 2:1 ratio
10. Existing assist counts preserved (players keep progress)

---

## 11. Performance & Scale Targets

**Response Times**:
- Referral link display: <100ms
- Coupon validation: <200ms
- Referral dashboard load: <2 seconds
- Token reward processing: <5 seconds (async, not blocking purchase)

**Throughput**:
- Support 1,000 referral clicks per day
- Handle 100 concurrent coupon redemptions
- Process 500 referral rewards per day

**Scalability**:
- Track 10,000+ referrals per trainer
- Support 1,000+ active coupons across all trainers
- Generate referral analytics for 100,000+ players

---

## 12. Questions / Open Issues

| ID | Question | Priority | Status | Owner |
|:---|:---|:---:|:---|:---|
| Q-06.10 | **Coupon eligibility**: Are coupons for new players only, existing players too, or trainer decides per coupon? | P1 | Open | Client |
| Q-06.11 | **Referral reward ratio**: What should the platform-wide rule be? (3:1? 5:1? 1:1?) Should referee also get welcome token? | P1 | Open | Client |
| Q-06.12 | **Attribution window**: 30-day assumption for referral link validity - confirm? | P2 | Open | Client |
| Q-06.13 | **Coupon stacking**: Can players use multiple coupons on one purchase? (Assume NO) | P2 | ✅ RESOLVED | Client |

**Resolutions**:
- **Q-06.13**: NO - Players can only use one coupon per purchase

**Coupon Implementation**:
- Coupons applied within platform (before Stripe)
- Platform calculates final discounted price
- Stripe receives final amount only (doesn't know about coupons)
- Coupon tracking and analytics handled in platform

---

## 13. Integration Points

**With Epic-01 (User Management)**:
- ShareLink system tracks referral attribution
- Player registration completes referral tracking
- Multi-trainer context affects referral links

**With Epic-05 (Payments)**:
- First purchase triggers referral reward
- Coupon codes apply discounts in-platform (Stripe receives final discounted amount)
- Token rewards credited to player-trainer balance
- Coupon discounts reflected in transaction logs

**With Epic-02 (Event Management)**:
- Coupons apply to event RSVPs
- Discounted prices processed at checkout

**With Epic-04 (LPPP Content)**:
- Coupons apply to content purchases
- Discounted content access processed

---

## 14. Acceptance Criteria (Epic Level)

**Referral System**:
- [ ] Every player has automatic referral link
- [ ] Referral tracking works across registration and first purchase
- [ ] Token rewards granted when assist count reaches threshold
- [ ] Platform-wide rule configurable by Super Admin
- [ ] Referral dashboard shows accurate stats

**Coupon System**:
- [ ] Trainers can create, edit, deactivate coupons
- [ ] Players can apply coupons at checkout
- [ ] Discounts calculated correctly (percentage and fixed)
- [ ] Usage limits enforced
- [ ] Expiration dates enforced
- [ ] Coupon analytics accurate

**Analytics**:
- [ ] Referral dashboard shows: conversions, top referrers, activity log
- [ ] Coupon analytics shows: usage, discount given, revenue impact
- [ ] Data updates in near real-time (<5 minutes delay acceptable)

**Performance**:
- [ ] Referral link load <100ms
- [ ] Coupon validation <200ms
- [ ] Dashboard load <2 seconds
- [ ] System handles 1,000 referrals per day

---

**Total User Stories**: 8

