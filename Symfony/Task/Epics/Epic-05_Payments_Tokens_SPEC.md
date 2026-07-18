# Epic-05: Payments & Tokens System

---

## 1. Epic Overview

### Purpose
Enable financial transactions between players/parents and trainers, including event payments, content purchases, and token-based payment system. Integrate with Stripe for payment processing and trainer payouts.

### Business Value
- **For Trainers**: Reliable payment collection, automated payouts, flexible pricing options
- **For Players/Parents**: Convenient payment methods, token pre-purchase, secure transactions
- **For Platform Owner**: Transaction fee revenue (5% default), subscription revenue ($15/month per trainer)

### Success Metrics
- ✅ 90%+ payment success rate (first attempt)
- ✅ <5 second payment processing time
- ✅ <1% payment dispute rate
- ✅ 80%+ of active trainers use tokens
- ✅ Average 3+ token purchases per player per month
- ✅ Zero PCI compliance issues (handled by Stripe)

---

## 2. Scope Summary

**Goals**:
- Process payments for events and content
- Support multiple payment methods (tokens, USD, subscriptions)
- Token system (pre-purchased credits, trainer-specific)
- 24-hour refund policy automation
- Stripe Connect integration for trainer payouts
- Payment method management (via Stripe)
- Transaction history per trainer context

**Non-Goals** (Post-MVP):
- Multi-currency support (USD only for MVP)
- Cryptocurrency payments
- Payment plans / installments
- Gift cards
- Platform wallet (beyond tokens)
- Trainer-to-coach payouts (in-platform)
- Invoice generation (trainers use Stripe)
- Tax calculation (trainers handle via Stripe)
- Token balance alerts (low balance notifications)
- Partial refunds (price change after RSVP)
- Subscription grace period (payment failure handling)

---

## 3. In Scope (MVP)

### Payment Methods
- [ ] Trainer-specific tokens (pre-purchased credits)
- [ ] USD payments (via Stripe Checkout)
- [ ] **Player Subscriptions - Unlimited Access**
  - **Implementation**: Special tokens that grant unlimited access
  - Trainer sets subscription price in platform
  - Can be activated from selected date after purchase
  - 30-day duration for unlimited access to trainer's qualifying events
  - RSVP restrictions: 1 event per day in advance, unlimited same-day
  - Payment via Stripe, subscription = unlimited access (not unlimited tokens)
- [ ] **Dual Pricing per Event**
  - Events can have BOTH USD price AND Token price
  - Players choose payment method (pay with money OR tokens)
  - Trainers can toggle either option per event
  - **Default**: Token pricing ENABLED (1 token), USD pricing DISABLED ($0)
- [ ] Free events ($0 price)

### Token System
- [ ] Token purchase per trainer (parent-trainer balance)
- [ ] **Flexible Token Pricing**
  - Events can cost multiple tokens (not always 1:1 with sessions)
  - Example: Premium 3-hour event = 2 tokens
  - Trainer-configurable per event
  - Each trainer sets their own token pricing
- [ ] Token usage for event RSVP and content access
- [ ] Token balance display per trainer context
- [ ] Token transaction history
- [ ] Token refunds (24-hour policy)
- [ ] **Trainer Can Gift Tokens**
  - Manual token additions without payment
  - Use cases: Refunds for edge cases, rewards for top performers, promotions
  - Permissions: Trainers ONLY (not coaches)
  - Requires audit logging (already in Epic-07)
  - UI: Trainer selects player, enters token amount, adds note

### Stripe Integration
- [ ] Stripe Connect (Express Accounts) for trainers
- [ ] Stripe Checkout for one-time payments
- [ ] Stripe Billing for trainer subscriptions ($15/month to Dale)
- [ ] Payment method storage (via Stripe Customer)
- [ ] Webhook handling (payment success, refunds)
- [ ] Application fee deduction (5% default, configurable)

### Payment Flows
- [ ] Event RSVP payment (tokens or USD)
- [ ] Content purchase payment (LPPP playlists)
- [ ] Token purchase flow
- [ ] Subscription purchase flow
- [ ] Refund processing (24-hour rule)

### Parent/Child Payments
- [ ] Parent manages all payment methods (Stripe Customer)
- [ ] Parent controls token balances (per trainer)
- [ ] Child RSVP triggers parent approval (see Epic-01)
- [ ] Parent completes payment for child

### Transaction Management
- [ ] Transaction history per trainer context
- [ ] Failed payment retry logic
- [ ] Refund tracking
- [ ] Receipt generation (via Stripe)

---

## 4. Out of Scope (Post-MVP)

**✅ CONFIRMED** (January 2026):
- ✅ **Player Subscriptions**: Special tokens activated from selected date after purchase
  - Implementation confirmed: Token-based (not separate Stripe subscription)
  - Trainer can activate subscription from any future date
  - Simple implementation using existing token system

**Phase 2 Deferrals**:
- ❌ Multi-currency (EUR, GBP, etc.)
- ❌ Payment plans / Buy Now Pay Later
- ❌ Platform-wide wallet (tokens are trainer-specific only)
- ❌ Gift cards / promo codes (coupons are in, but not gift cards)
- ❌ **Trainer-to-coach payment distribution** (trainers pay coaches outside platform)
  - Platform provides analytics (sessions covered, player counts, attendance) for trainer's records
  - Trainers handle coach compensation independently
  - See Q-05.08 for future consideration
- ❌ Invoice generation (trainers use Stripe Dashboard)
- ❌ Recurring token auto-purchase
- ❌ Token expiration dates
- ❌ Fractional tokens (e.g., 0.5 tokens)
- ❌ Token transfer between trainers
- ❌ Refund requests (manual, not automated approval)

---

## 5. Dependencies

### Required Before This Epic
- **Epic-01** (User Management) - Players, trainers, parent accounts must exist
- **Epic-02** (Event Management) - Events must exist to purchase
- **Epic-04** (LPPP Content) - Content must exist to purchase

### Blocks These Epics
- None (can be developed in parallel with Epic-06, Epic-07)

### External Dependencies
- **Stripe Platform**: Payments, Connect, Billing, Webhooks
- **Email Service**: Payment confirmation, receipt emails

---

## 6. User Roles Involved

| Role | Interaction | Permissions |
|:---|:---|:---|
| **Trainer / Business Owner** | Sets pricing, manages Stripe Connect, views payout history | Can set token prices, event prices, subscription prices; views own financial data in Stripe |
| **Coach / Contractor** | No payment interaction (paid outside platform) | No payment permissions |
| **Player / Parent** | Makes payments, purchases tokens, manages payment methods (via Stripe) | Can purchase for self or children, view transaction history per trainer |
| **Super Admin** | Configures platform fees, monitors transaction health, accesses all Stripe data | Full access to all financial data, can adjust per-trainer fee rates |

---

## 7. User Stories & Acceptance Criteria

### US-05.01: Trainer Connects Stripe Account

**As a** Trainer  
**I want to** connect my Stripe account  
**So that** I can receive payments from my players

**Acceptance Criteria**:
- [ ] From Trainer Settings, click "Connect Stripe"
- [ ] Redirected to Stripe Connect onboarding (Express Account)
- [ ] Complete Stripe KYC process (5-10 minutes):
  - Business/personal info
  - Bank account for payouts
  - Tax information (W-9 or W-8)
- [ ] After completion, redirected back to platform
- [ ] Status shown: "Stripe Connected ✓"
- [ ] Can now create paid events and accept payments

**Stripe Configuration**:
- Express Account type (simplified onboarding)
- Application fee: 5% default (configurable per trainer by Super Admin)
- Payout schedule: Monthly, 1st of month (configurable to weekly if needed)
- Trainer subscription: $15/month via Stripe Billing

**If Trainer Not Connected**:
- [ ] Cannot create paid events (blocked with message: "Connect Stripe first")
- [ ] Cannot sell content (blocked)
- [ ] Can still create free events

---

### US-05.02: Player Purchases Tokens

**As a** Player or Parent  
**I want to** buy tokens from my trainer  
**So that** I can pay for sessions quickly without entering card details each time

**Acceptance Criteria**:
- [ ] From current trainer context (e.g., "Sarah → Coach Bob")
- [ ] Navigate to "Tokens" or "Wallet" section
- [ ] See current token balance: "You have 5 tokens with Coach Bob"
- [ ] Click "Buy Tokens"
- [ ] Select token package:
  - 10 tokens - $90 (save 10%)
  - 25 tokens - $225 (save 10%)
  - 50 tokens - $450 (save 10%)
  - Custom amount
- [ ] Trainer sets token price (e.g., 1 token = $10)
- [ ] Click "Purchase"
- [ ] Redirected to Stripe Checkout
- [ ] Complete payment
- [ ] Redirected back to platform
- [ ] Token balance updated immediately
- [ ] Confirmation email sent with receipt

**Parent-Trainer Token Balance** (Key Architecture Decision):
- Tokens stored at **parent-trainer** relationship level
- Example: Parent Sarah buys 50 tokens from Coach Bob
- Those 50 tokens can be used for ANY of Sarah's children who train with Coach Bob (Alex, Maya, Emma)
- **NOT** per-child balances (simpler model)

**Token Purchase from Child View**:
- [ ] If child (with login) views tokens, they see parent's token balance
- [ ] Child can initiate token purchase (triggers parent approval - see Epic-01 US-01.05)
- [ ] After parent approval, purchase completed
- [ ] Tokens added to parent-trainer balance

---

### US-05.03: Player Pays for Event with Tokens

**As a** Player or Parent  
**I want to** use tokens to RSVP for an event  
**So that** I can quickly register without payment delays

**Acceptance Criteria**:
- [ ] Player/parent views event: "Basketball Skills - 2 tokens or $20"
- [ ] Current token balance shown: "You have 5 tokens"
- [ ] Click "RSVP with Tokens"
- [ ] Confirmation: "Use 2 tokens for [Event]?"
- [ ] Confirm
- [ ] Token balance deducted: 5 → 3 tokens
- [ ] RSVP confirmed instantly (no payment processing delay)
- [ ] Email confirmation sent

**If Insufficient Tokens**:
- [ ] Show: "You have 1 token, need 2 tokens"
- [ ] Options:
  - "Buy More Tokens" (redirects to token purchase)
  - "Pay with Card Instead" ($20 via Stripe Checkout)

**Parent RSVPing Child**:
- [ ] Parent in context "Maya → Coach Bob", token balance: 10 tokens
- [ ] Parent RSVPs Maya to event (2 tokens)
- [ ] Parent's token balance with Coach Bob: 10 → 8 tokens
- [ ] Maya registered for event

---

### US-05.04: Player Pays for Event with USD

**As a** Player or Parent  
**I want to** pay for an event with my credit card  
**So that** I can register even without tokens

**Acceptance Criteria**:
- [ ] Player/parent views event: "Basketball Skills - $20 or 2 tokens"
- [ ] Click "RSVP"
- [ ] Select payment method: "💳 Pay with Card" or "🪙 Pay with Tokens"
- [ ] If "Pay with Card" selected:
  - [ ] Redirected to Stripe Checkout
  - [ ] Enter card details (or use saved payment method)
  - [ ] Complete payment
  - [ ] Stripe processes $20:
    - Platform fee: $1 (5%)
    - Stripe fee: ~$0.88 (2.9% + $0.30)
    - Trainer receives: ~$18.12 (net)
  - [ ] Redirected back to platform
  - [ ] RSVP confirmed
  - [ ] Email confirmation with receipt

**Payment Method Storage**:
- [ ] After first payment, Stripe saves payment method to customer
- [ ] Next time: "Use card ending in 4242" option shown
- [ ] Can add/remove payment methods via Stripe Customer Portal

**Child Payment with Parent Approval** (see Epic-01 US-01.05):
- [ ] Child clicks RSVP → Payment required
- [ ] Status: "Pending Parent Approval"
- [ ] Parent receives notification
- [ ] Parent approves → Stripe Checkout opened for parent
- [ ] Parent completes payment → Child registered

---

### US-05.05: Player Cancels RSVP and Receives Refund

**As a** Player or Parent  
**I want to** cancel my RSVP and get refund  
**So that** I can recover my payment if plans change

**24-Hour Refund Policy**:

**Scenario A: Cancel 24+ Hours Before Event**:
- [ ] Player cancels RSVP
- [ ] System checks: Event in 3 days (>24 hours)
- [ ] Automatic refund:
  - If paid with tokens: Tokens returned to balance
  - If paid with USD: Stripe refund initiated
- [ ] Confirmation: "Full refund processed ($20 or 2 tokens returned)"
- [ ] Email sent with refund confirmation

**Scenario B: Cancel <24 Hours Before Event**:
- [ ] Player cancels RSVP
- [ ] System checks: Event in 12 hours (<24 hours)
- [ ] Confirmation: "No refund available (within 24-hour window). Continue?"
- [ ] If confirmed: RSVP canceled, no refund issued
- [ ] Tokens/payment forfeited

**Scenario C: Trainer Cancels Event**:
- [ ] Trainer cancels event (see Epic-02)
- [ ] System automatically refunds ALL registered players:
  - Tokens returned to player balances
  - USD payments refunded via Stripe
- [ ] Regardless of timing (always full refund if trainer cancels)

**Refund Processing**:
- [ ] Token refunds: Instant (balance updated immediately)
- [ ] USD refunds: 5-10 business days (Stripe standard)
- [ ] Refund history tracked in transaction log

---

### US-05.06: Player Purchases Content (LPPP Playlist)

**As a** Player or Parent  
**I want to** buy access to training content  
**So that** I can learn and practice between sessions

**Acceptance Criteria** (Paywall Model - D-SCOPE-011):
- [ ] Player browses LPPP content library for current trainer
- [ ] Sees playlist: "Ball Handling Mastery - $50 one-time or 5 tokens"
- [ ] Playlist shown as 🔒 Locked (cannot access)
- [ ] Click "Purchase Access"
- [ ] Select payment method:
  - 💳 Pay $50 with Card
  - 🪙 Pay 5 Tokens
- [ ] If Card: Stripe Checkout flow
- [ ] If Tokens: Instant deduction from balance
- [ ] After payment: Playlist unlocked
- [ ] Player can now view all videos in playlist
- [ ] Access persists forever (one-time purchase)

**Alternative Pricing Models** (Q-05.02 - Not Confirmed):
- Subscription model (monthly access to all content)
- Bundle pricing (multiple playlists at discount)
- Event + content bundles
- **Status**: Needs Dale approval before implementation

**Trainer Content Suggestions**:
- [ ] Trainer can "Suggest" playlist to specific player
- [ ] Player receives notification: "Coach Bob suggests: Ball Handling Mastery"
- [ ] Player still must purchase to access
- [ ] Suggestion appears highlighted in player's content library

---

### US-05.07: Parent Manages Payment Methods

**As a** Parent  
**I want to** manage my saved payment methods  
**So that** I can control which cards are used for family payments

**Acceptance Criteria**:
- [ ] Navigate to "Settings" → "Payment Methods"
- [ ] Click "Manage Payment Methods" → **Redirects to Stripe Customer Portal**
- [ ] Stripe Customer Portal allows parents to:
  - View all saved payment methods
  - Add new cards
  - Set default payment method
  - Remove cards
- [ ] Changes saved in Stripe (synced automatically)
- [ ] Default card used for all future purchases (events, tokens, content)

**Implementation Note**:
- [ ] Payment method management handled entirely via **Stripe Customer Portal** (no custom UI)
- [ ] Each parent/player account has ONE Stripe Customer ID
- [ ] Payment methods stored at Stripe (not on our platform - PCI compliant)
- [ ] Same cards used across all trainers

---

### US-05.08: Player Views Transaction History

**As a** Player or Parent  
**I want to** see my payment history  
**So that** I can track spending and verify charges

**Acceptance Criteria**:
- [ ] Navigate to "Transactions" or "Payment History"
- [ ] **Context-Based Display**: Shows transactions for current trainer context only
- [ ] Example: Parent in "Maya → Coach Bob" context sees:
  - All token purchases from Coach Bob
  - All event payments for Maya with Coach Bob
  - All content purchases for Maya with Coach Bob
  - All refunds related to Coach Bob

**Transaction List Shows**:
- [ ] Date & time
- [ ] Type (Token Purchase, Event RSVP, Content Purchase, Refund)
- [ ] Description ("Basketball Skills - Maya", "10 Tokens")
- [ ] Amount (-$20, +2 tokens, etc.)
- [ ] Payment method (Visa 4242, Tokens)
- [ ] Status (Completed, Pending, Refunded, Failed)
- [ ] Receipt link (Stripe-generated)

**Filters**:
- [ ] Date range (Last 30 days, Last 3 months, Last year, Custom)
- [ ] Transaction type (All, Purchases, Refunds)
- [ ] Payment method (All, Card, Tokens)

**Parent "Family Overview" (Optional)**:
- [ ] Parent dashboard can show "Recent Transactions - All Trainers"
- [ ] Shows last 10 transactions across all children and trainers
- [ ] Click to filter by specific child-trainer context

---

### US-05.09: Trainer Views Payout History

**As a** Trainer  
**I want to** see my payout history  
**So that** I can track my earnings

**Acceptance Criteria**:
- [ ] Navigate to "Payments" or "Earnings"
- [ ] Message: "View full financial reports in your Stripe Dashboard"
- [ ] Link to Stripe Express Dashboard
- [ ] Platform shows simplified summary:
  - Current period earnings (since last payout)
  - Next payout date (1st of next month)
  - Last payout amount
  - Total lifetime earnings

**Stripe Dashboard** (Source of Truth - D-ARCH-001):
- [ ] Trainers use Stripe Express Dashboard for:
  - Detailed transaction history
  - Payout history and bank details
  - Tax documents (1099s)
  - Refund management
  - Fee breakdown
- [ ] Platform links to Stripe, does not duplicate financial reporting

---

### US-05.10: Super Admin Configures Per-Trainer Fee

**As a** Super Admin  
**I want to** set custom fee rates for specific trainers  
**So that** I can offer special pricing to partners or early adopters

**Acceptance Criteria**:
- [ ] From Super Admin dashboard, navigate to "Trainers"
- [ ] Select trainer: "Coach Bob"
- [ ] View current settings:
  - Monthly subscription: $15/month
  - Application fee: 5% (platform default)
- [ ] Click "Edit Pricing"
- [ ] Modify:
  - Monthly subscription: $10/month (promotional pricing)
  - Application fee: 3% (reduced rate)
- [ ] Save changes
- [ ] Trainer's next billing cycle uses new rates
- [ ] New transactions use 3% fee instead of 5%

**Audit Logging**:
- [ ] All pricing changes logged
- [ ] Shows: Who changed, when, old rate, new rate, reason (optional)

---

## 8. Data Requirements

### What Information Needs to Be Stored

**For Token Balances**:
- Parent/Player account reference
- Trainer reference
- Current balance (integer)
- Last updated timestamp

**For Token Transactions**:
- Transaction unique identifier
- Parent/Player account
- Trainer
- Type (Purchase, Usage, Refund)
- Amount (tokens)
- Reason ("Event RSVP: Basketball Skills", "Token Purchase", "Refund")
- Timestamp
- Related event or content (if applicable)

**For USD Transactions**:
- Transaction unique identifier
- Parent/Player account
- Trainer
- Stripe Payment Intent ID (reference to Stripe)
- Amount (USD)
- Type (Event Payment, Content Purchase, Subscription)
- Status (Pending, Completed, Failed, Refunded)
- Related event or content (if applicable)
- Timestamp

**For Stripe Customer**:
- Parent/Player account reference
- Stripe Customer ID
- Default payment method ID (optional)

**For Trainer Stripe Connect**:
- Trainer account reference
- Stripe Connect Account ID
- Onboarding status (Pending, Complete, Incomplete)
- Subscription status (Active, Past Due, Canceled)
- Custom fee rate (if different from default 5%)
- Connected timestamp

**For Refunds**:
- Refund unique identifier
- Original transaction reference
- Refund amount (USD or tokens)
- Refund reason (Player canceled, Trainer canceled, Manual)
- Refund timestamp
- Stripe Refund ID (if USD refund)

---

## 9. Business Rules & Logic

### Token System Rules

**Token Pricing**:
- Trainer sets token price (e.g., 1 token = $10)
- Token packages offer discounts (e.g., 10 tokens = $90 instead of $100)
- Tokens are trainer-specific (cannot be used with different trainers)

**Token Balance Storage**:
- **Parent-Trainer Balance Model**: Tokens stored at parent-trainer relationship level
- Example: Parent Sarah trains 3 children (Alex, Maya, Emma) with Coach Bob
- Sarah buys 50 tokens from Coach Bob
- All 3 children can use those 50 tokens when RSVPing with Coach Bob
- Simpler than per-child balances

**Token Usage Priority**:
- When event allows both tokens and USD, player chooses payment method
- No automatic token usage (player explicitly selects)

**Token Expiration**:
- Tokens do NOT expire for MVP (Phase 2 may add expiration if needed)

**Token Refunds**:
- Refunded tokens return to parent-trainer balance immediately
- Partial refunds possible (e.g., event was 2 tokens, refund 2 tokens)

### Payment Processing Rules

**Stripe Checkout Flow**:
- All USD payments go through Stripe Checkout (hosted page)
- Stripe handles card validation, 3D Secure, fraud detection
- After payment, user redirected back to platform
- Webhook confirms payment success asynchronously

**Application Fee Deduction**:
- Fee included in price (trainer absorbs)
- Example: Event priced at $100
  - Player pays $100
  - Stripe fee: ~$3.20 (2.9% + $0.30)
  - Platform fee: $5 (5% application fee)
  - Trainer receives: ~$91.80 net
- Trainer sees full breakdown in Stripe Dashboard

**Failed Payments**:
- If payment fails: RSVP not confirmed, spot remains available
- Player notified of failure
- Retry option: "Try Again" button
- After 3 failed attempts: Suggest different payment method

**Pending Payments**:
- Some payment methods take time (bank transfers, etc.)
- RSVP status: "Payment Pending"
- Webhook updates status when payment clears
- If payment fails after 7 days: RSVP auto-canceled

### Refund Policy Rules

**24-Hour Rule**:
- Cancel ≥24 hours before event: Full refund
- Cancel <24 hours before event: No refund
- Trainer cancels event: Always full refund (regardless of timing)

**Refund Processing**:
- Token refunds: Instant
- USD refunds: Initiated immediately, takes 5-10 business days
- Stripe handles actual refund processing

**Trainer Manual Override**:
- Trainers can manually issue refunds via Stripe Dashboard
- For special cases (injury, family emergency, etc.)
- Platform does not enforce manual refund rules (trainer discretion)

### Subscription Rules

**Trainer Subscription** (to Platform Owner):
- $15/month default
- Billed 1st of each month via Stripe Billing
- If payment fails: 3 retry attempts over 10 days
- If still fails: Trainer account suspended (cannot create paid events)
- Upon payment: Account reactivated

**Player Subscriptions**:
- 30-day unlimited access to trainer's qualifying events
- Implemented as special tokens activated from selected date
- Trainer sets subscription price in platform
- Payment processed via Stripe
- Subscription token granted only after successful payment (no grace period)
- RSVP restrictions: 1 event per day in advance, unlimited same-day

### Multi-Tenant Payment Rules

**Parent with Multiple Trainers**:
- Separate token balances per trainer
- Example: Sarah has 10 tokens with Coach Bob, 5 tokens with Coach Lisa
- Cannot mix tokens between trainers

**Payment Methods**:
- Shared across all trainers (Stripe Customer level)
- Parent uses same cards for all children and trainers

**Transaction Isolation**:
- Transaction history scoped to trainer context
- Trainer A cannot see player's transactions with Trainer B

---

## 10. User Flows

### Flow 1: Player Purchases Tokens and RSVPs with Tokens

1. Player logs in, context: "Coach Bob"
2. Navigate to "Tokens" section
3. Current balance: 0 tokens
4. Click "Buy Tokens"
5. Select package: "25 tokens - $225 (save 10%)"
6. Click "Purchase"
7. Redirected to Stripe Checkout
8. Enter card details (Visa ending 4242)
9. Complete payment
10. Redirected back to platform
11. Token balance updated: 0 → 25 tokens
12. Confirmation email received
13. Navigate to event: "Basketball Skills - 2 tokens or $20"
14. Click "RSVP with Tokens"
15. Confirmation: "Use 2 tokens?"
16. Confirm
17. Token balance: 25 → 23 tokens
18. RSVP confirmed instantly
19. Email confirmation received

### Flow 2: Parent RSVPs Child to Event with USD

1. Parent logs in, context: "Maya → Coach Bob"
2. Browse events
3. Click event: "Basketball Skills - $20"
4. Click "RSVP"
5. Select "💳 Pay with Card"
6. Redirected to Stripe Checkout
7. Select saved payment method: "Visa ending 4242"
8. Click "Pay $20"
9. Stripe processes payment
10. Redirected back to platform
11. RSVP confirmed for Maya
12. Email confirmation with receipt sent to parent
13. Maya appears in event roster

### Flow 3: Player Cancels RSVP 3 Days Before Event (Refund)

1. Player views "My Reservations"
2. Sees upcoming event: "Basketball Skills" (in 3 days, paid $20)
3. Click "Cancel RSVP"
4. Confirmation: "Event is in 3 days. You'll receive a full refund ($20). Continue?"
5. Confirm cancellation
6. RSVP canceled
7. Stripe refund initiated automatically
8. Confirmation: "Your $20 refund will appear in 5-10 business days"
9. Email sent with refund confirmation
10. If paid with tokens: Tokens returned immediately (balance updated)

### Flow 4: Parent Purchases Content for Child

1. Parent in context: "Alex → Coach Bob"
2. Navigate to "LPPP" content library
3. Browse playlists
4. Click locked playlist: "Ball Handling Mastery - $50 or 5 tokens"
5. Click "Purchase Access"
6. Token balance: 10 tokens available
7. Select "🪙 Pay 5 Tokens"
8. Confirmation: "Use 5 tokens to unlock Ball Handling Mastery for Alex?"
9. Confirm
10. Token balance: 10 → 5 tokens
11. Playlist unlocked for Alex
12. Alex can now view all videos in playlist
13. Confirmation email sent

### Flow 5: Trainer Connects Stripe and Receives First Payout

1. Trainer creates account (Super Admin creates)
2. Trainer logs in first time
3. Prompt: "Connect Stripe to accept payments"
4. Click "Connect Stripe"
5. Redirected to Stripe Connect onboarding
6. Complete form:
   - Business name: "Bob's Basketball Academy"
   - Tax info: EIN 12-3456789
   - Bank account: XXX-XXXX-1234
7. Submit for review
8. Stripe approves (instant or up to 48 hours)
9. Redirected back to platform: "Stripe Connected ✓"
10. Trainer creates first paid event: "Basketball Skills - $20"
11. Players RSVP and pay
12. Trainer earns $200 in first month
13. On 1st of next month: $200 payout initiated to bank account
14. Payout arrives in 2-3 business days
15. Trainer views payout in Stripe Dashboard

---

## 11. Performance & Scale Targets

**Response Times**:
- Token balance check: <100ms
- Token purchase redirect to Stripe: <500ms
- Payment confirmation webhook processing: <2 seconds
- Refund processing: <5 seconds (token), <10 seconds (USD initiation)

**Throughput**:
- Support 100 concurrent payments
- Handle 1,000 transactions per day (Year 1 estimate)
- Process 50 webhooks per minute

**Reliability**:
- 99.9% payment processing uptime (relies on Stripe SLA)
- <0.1% payment data loss
- Webhook retry: Up to 3 attempts over 24 hours

**Stripe Dependencies**:
- All financial processing handled by Stripe (no custom payment logic)
- Platform must handle Stripe API rate limits (100 requests/second)
- Webhook signature verification required (security)

---

## 12. Questions / Open Issues

| ID | Question | Priority | Status | Owner |
|:---|:---|:---:|:---|:---|
| Q-05.01 | Refund policy finalized: 24-hour window | P0 | ✅ RESOLVED | Client |
| Q-05.02 | **Player subscriptions**: Should trainers be able to offer monthly subscription for unlimited content/event access? Or one-time purchases only for MVP? | P1 | ✅ RESOLVED | Client |
| Q-05.03 | Token package sizes and discounts: What's the pricing strategy? | P2 | Open | Client |
| Q-05.04 | Failed payment retry: How many attempts? Auto-email reminders? | P2 | ✅ RESOLVED | Team |
| Q-05.05 | Partial refunds: If event price changes after RSVP, refund difference? | P2 | ✅ RESOLVED | Client |
| Q-05.06 | Token balance alerts: Notify player when balance low? | P2 | ✅ RESOLVED | Client |
| Q-05.07 | Subscription grace period: How long until access cut off after failed payment? | P2 | ✅ RESOLVED | Client |
| Q-05.08 | **Trainer-to-coach payments**: Should platform facilitate coach payment distribution in future? Or always handled outside platform? (Assume OUT for MVP, provide analytics only) | P2 | ✅ RESOLVED | Client |

**Resolutions**:
- **Q-05.04**: Stripe handles payment retry logic automatically
- **Q-05.05**: Too complex for MVP - trainers can use token gifting to handle edge cases
- **Q-05.06**: NO - token balance alerts not needed for MVP
- **Q-05.07**: No grace period - subscription token granted only after successful payment (immediate access on payment, no access on failed payment)
- **Q-05.08**: OUT of MVP - trainer-to-coach payments handled externally, platform provides analytics only

---

## 13. Integration Points

**With Epic-02 (Event Management)**:
- Event RSVP triggers payment flow
- Event cancellation triggers refund flow
- Event capacity checks payment status before confirmation
- Trainer-canceled events trigger automatic refunds

**With Epic-04 (LPPP Content)**:
- Content purchase triggers payment flow
- Content access gates verify payment/subscription status
- Trainer content suggestions link to purchase flow

**With Epic-01 (User Management)**:
- Child actions trigger parent approval workflow
- Parent manages payment methods via Stripe (for whole family)
- Stripe Customer ID stored at parent account level
- Multi-trainer context affects token balance display

**With Epic-06 (Marketing Tools)**:
- Referral rewards trigger token grants
- Coupon codes apply discounts to payments
- First purchase triggers referral reward

**With Stripe**:
- Stripe Connect for trainer accounts
- Stripe Checkout for payment processing
- Stripe Billing for trainer subscriptions (trainers pay platform owner $15/month)
- Stripe Webhooks for event updates (payment success, refund, trainer subscription changes)

---

## 14. Technical Notes

### Stripe Webhook Events to Handle
- `payment_intent.succeeded` → Confirm RSVP, unlock content
- `payment_intent.payment_failed` → Notify player, retry payment
- `charge.refunded` → Update transaction history, restore tokens
- `customer.subscription.created` → Grant subscription access
- `customer.subscription.deleted` → Revoke subscription access
- `account.updated` → Update trainer Stripe connection status

### Security Requirements
- Webhook signature verification (prevent spoofing)
- Idempotency keys for payment operations (prevent duplicate charges)
- Never store card details on platform (PCI compliance)
- Encrypt Stripe API keys in environment variables
- Rate limit payment attempts (prevent brute force)

### Error Handling
- Stripe API errors: Log and notify admin
- Payment failures: Clear user-friendly messages
- Webhook failures: Retry with exponential backoff
- Token balance insufficient: Prompt to purchase more
- Network failures during Stripe Checkout: Resume on return

### Testing Requirements
- Use Stripe Test Mode for development
- Test cards: 4242 4242 4242 4242 (success), 4000 0000 0000 9995 (decline)
- Test webhooks with Stripe CLI
- Test refund scenarios (24-hour boundary cases)
- Test parent-child payment approval workflow

---

## 15. Acceptance Criteria (Epic Level)

**Payment Processing**:
- [ ] Player can pay for events with tokens
- [ ] Player can pay for events with credit card
- [ ] Parent can purchase tokens for trainer (usable by all children)
- [ ] Token balance displayed correctly per trainer context
- [ ] Refunds processed automatically per 24-hour rule

**Stripe Integration**:
- [ ] Trainers can connect Stripe Express accounts
- [ ] Platform receives 5% application fee (configurable per trainer)
- [ ] Trainers receive monthly payouts
- [ ] Webhooks processed reliably (99%+ success rate)

**Parent/Child Payments**:
- [ ] Parent manages payment methods via Stripe (used for all children)
- [ ] Parent controls token balances per trainer
- [ ] Child actions trigger parent approval (Epic-01 integration)
- [ ] Parent can complete payment for child

**Transaction History**:
- [ ] Transaction history visible per trainer context
- [ ] Token transactions tracked (purchase, usage, refund)
- [ ] USD transactions linked to Stripe receipts
- [ ] Refunds clearly marked in history

**Content Payments** (Epic-04 integration):
- [ ] Players can purchase LPPP playlists
- [ ] Payment gates content access
- [ ] Purchased content remains accessible

**Performance**:
- [ ] Payment processing <5 seconds
- [ ] Token balance updates instant
- [ ] System handles 100 concurrent payments
- [ ] Zero PCI compliance issues (Stripe handles)

---

**Total User Stories**: 10

