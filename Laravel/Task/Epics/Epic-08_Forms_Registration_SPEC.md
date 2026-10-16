# Epic-08: Forms & Registration (Camps/Evaluations)

---

## 1. Description

**Epic-08** implements a **forms-based registration system** for camps and evaluations—distinct from calendar-based events. This epic enables trainers to collect player information, process payments, and convert form submissions into full user accounts.

**Key Distinction**: Camps and Evaluations are **FORMS**, not calendar events:
- **Forms**: Data collection tools with shareable links
- **Calendar Events** (Epic-02): Time-scheduled activities with RSVP

**Core Features**:
- Simple form builder with pre-loaded templates
- Shareable external links (social media, website, email)
- Capacity limits
- Optional Stripe payment integration
- Player conversion to full user accounts

**Business Value**: 
- **Lead Generation**: Capture new players through camps
- **Revenue**: Monetize camps and evaluations
- **Conversion**: Transform camp participants into regular platform users
- **Flexibility**: Evaluations always available, camps can be temporary

---

## 2. Business Value

**Problem Statement**: 
Trainers need to market camps externally and collect registrations from non-users. Current platform only serves existing players. Trainers need a way to:
- Promote camps on social media
- Collect registration info from people who aren't platform users yet
- Process payments for camps
- Convert camp participants into regular customers

**Success Metrics**:
- ✅ 70%+ of trainers create at least one camp/evaluation form
- ✅ Average 2-3 camps per trainer per quarter
- ✅ 30%+ conversion rate (camp participant → full user account)
- ✅ Camp registrations account for 20%+ of new player acquisitions
- ✅ Zero payment processing errors for camp purchases

---

## 3. In Scope (MVP)

###  Forms & Registration System
- [ ] Form builder with pre-loaded templates (Camps, Evaluations)
- [ ] Form fields: Text inputs, Dropdowns (single-select), Multi-select
- [ ] Form customization (add/remove/reorder fields)
- [ ] Form preview before publishing
- [ ] Shareable external links
- [ ] Capacity limits (max participants)
- [ ] Optional pricing (Stripe payment on submission)
- [ ] Turn on/off capability (Camps can be temporary)

### Camps (Temporary Forms)
- [ ] Create camp form
- [ ] Set camp dates (for display purposes, not calendar-linked)
- [ ] Set capacity limit
- [ ] Set price (optional)
- [ ] Turn camp on/off (enable/disable registration)
- [ ] Share camp link externally
- [ ] View camp registrations
- [ ] Export camp participant list

### Evaluations (Always-On Forms)
- [ ] Create evaluation form
- [ ] Always available on trainer's portal (permanent link)
- [ ] Set price (optional)
- [ ] View evaluation submissions
- [ ] Export evaluation submissions

### Form Submission Flow
- [ ] Non-user clicks shareable link
- [ ] Views camp/evaluation details
- [ ] Fills form fields
- [ ] If price set: Redirected to Stripe Checkout
- [ ] Payment processed (if applicable)
- [ ] Registration confirmed
- [ ] Submission stored in platform
- [ ] Offer user account creation (convert to full user)

### Trainer Experience
- [ ] Camp/Evaluation management dashboard
- [ ] View all form submissions
- [ ] Filter submissions by status (Paid, Pending, Converted to User)
- [ ] Mark attendance (for camps)
- [ ] Send bulk messages to camp participants
- [ ] Convert submission to user account (manual trigger if player doesn't self-convert)

### Player Conversion Flow (Integration with Epic-01)
- [ ] After form submission: "Create your account to access more features"
- [ ] Player creates password
- [ ] Account created with form data pre-populated
- [ ] Linked to trainer automatically
- [ ] OR: Send ShareLink via email for later registration

---

## 4. Out of Scope (Post-MVP)

**Phase 2 Deferrals**:
- ❌ Complex form fields (file uploads, images, signature pads, tables)
- ❌ Conditional logic (show/hide fields based on answers)
- ❌ Multi-page forms (all fields on one page for MVP)
- ❌ Form analytics (completion rates, drop-off points)
- ❌ Email reminders for incomplete registrations
- ❌ Refund processing for camps (handled manually via Stripe for MVP)
- ❌ Waitlist for camps (show "Full" only)
- ❌ Early bird pricing (single price only)
- ❌ Group discounts (family/team pricing)
- ❌ Recurring evaluations (evaluation requests from players)
- ❌ Evaluation scheduling (time slot selection for in-person evaluations)
- ❌ Video upload in evaluation forms (YouTube link only if needed)
- ❌ Form versioning (edit history)
- ❌ Form duplication/templates beyond pre-loaded
- ❌ Integration with external form tools (Typeform, Google Forms)

**Simplifications**:
- Basic form builder (not drag-and-drop)
- Template-based approach (pre-loaded Camps & Evaluations templates)
- Simple field types only
- No complex validation rules

---

## 5. Dependencies

### Required Before This Epic
- **Epic-01** (User Management) - For user account creation flow
- **Epic-05** (Payments) - For Stripe Checkout integration

### This Epic Enables
- Trainer lead generation
- Camp revenue
- Player acquisition funnel

### Integration Points
- **→ Epic-01**: Camp-to-user conversion flow
- **→ Epic-05**: Stripe payment for paid camps/evaluations
- **→ Epic-07**: Feature toggle (Camps can be disabled per trainer)

---

## 6. Assumptions

### Business Assumptions
1. Most camps have pricing (revenue-generating)
2. Evaluations are often free (lead generation)
3. External link sharing is primary distribution method
4. Conversion to user account is optional (not forced)
5. Camp participants expect simple, fast registration process

### Technical Assumptions
1. Form data stored in platform database
2. Stripe Checkout handles payment processing (not custom payment form)
3. Form links are publicly accessible (no login required)
4. Capacity limits enforced at submission time (not live countdown)
5. Basic form validation (required fields, email format)

### UX Assumptions
1. Forms are mobile-friendly (primary device for submissions)
2. One-page forms (not multi-step)
3. Form completion takes < 5 minutes
4. Clear progress indicators for payment steps

---

## 7. User Stories

### US-08.01: Trainer Creates Camp Form
**As a** trainer  
**I want to** create a camp registration form  
**So that** I can market my camp externally and collect registrations

**Acceptance Criteria**:
- Trainer accesses "Camps & Evaluations" section
- Selects "Create Camp" → Pre-loaded template appears
- Customizes form fields (add/remove/reorder)
- Sets camp name, dates (display only), description
- Sets capacity limit (e.g., max 50 participants)
- Sets price (optional, integrated with Stripe)
- Preview form before publishing
- Publishes form → Receives shareable link
- Can turn camp on/off (enable/disable registration)

**Business Rules**:
- Minimum 1 field required (participant name)
- Capacity limit: 1-1000 participants
- Price: $0 (free) or $1-$10,000 range
- Dates are display-only (not linked to calendar)

---

### US-08.02: Trainer Creates Evaluation Form
**As a** trainer  
**I want to** create an always-available evaluation form  
**So that** prospective players can request evaluations anytime

**Acceptance Criteria**:
- Trainer accesses "Camps & Evaluations" section
- Selects "Create Evaluation" → Pre-loaded template appears
- Customizes form fields
- Sets evaluation name, description
- Sets price (optional)
- Preview form
- Publishes form → Receives permanent link
- Evaluation remains active indefinitely (no on/off toggle)

**Business Rules**:
- Evaluations are always accessible (cannot be disabled, only deleted)
- No capacity limits for evaluations
- Same form field types as camps

---

### US-08.03: Non-User Submits Camp Form
**As a** non-user (parent/player)  
**I want to** register for a camp via shared link  
**So that** I can secure my spot and pay for camp

**Acceptance Criteria**:
- Clicks shareable camp link → Form loads
- Views camp details (name, dates, description, price, spots remaining)
- Fills form fields (name, email, age, emergency contact, etc.)
- Reviews waiver/terms (if included in form)
- Submits form:
  - **If free**: Registration confirmed immediately
  - **If paid**: Redirected to Stripe Checkout → Pays → Registration confirmed
- Receives confirmation email with camp details
- Offered option to create user account ("Create account to access training calendar")

**Business Rules**:
- If capacity reached: Form shows "Camp Full" message (no submission allowed)
- Email validation required
- Payment must complete for paid camps (pending payments = not registered)
- Partial submissions not saved (all fields required before submit)

---

### US-08.04: Trainer Views Camp Submissions
**As a** trainer  
**I want to** view all camp registrations  
**So that** I can track attendance and manage participants

**Acceptance Criteria**:
- Trainer accesses camp details
- Views participant list with columns:
  - Name, Email, Age, Payment Status, Submission Date, Converted to User (Yes/No)
- Filters by:
  - Payment Status (Paid, Free, Pending)
  - Converted Status (User Created, Not Converted)
- Exports participant list (CSV)
- Can mark attendance (checkboxes for each participant)
- Can send bulk message to all participants (email)

**Business Rules**:
- Only paid/free registrations shown (pending payments excluded)
- Participant data visible indefinitely (even after camp ends)

---

### US-08.05: Player Converts Camp Submission to Account
**As a** camp participant  
**I want to** create a full user account  
**So that** I can access training calendar and other platform features

**Acceptance Criteria**:
- After camp registration: Sees "Create Your Account" prompt
- Clicks prompt → Registration form with pre-filled data (name, email)
- Sets password
- Accepts terms
- Account created → Auto-assigned to trainer
- Redirected to player dashboard
- Can now RSVP for events, purchase content, etc.

**Alternative Flow**:
- Player skips account creation → Receives email with ShareLink
- Can create account later using ShareLink

**Business Rules**:
- Camp submission email must not already be in system (duplicate check)
- If email exists: Prompt to login instead
- Account creation is optional (not forced)

---

### US-08.06: Trainer Manages Form Settings
**As a** trainer  
**I want to** edit and manage my forms  
**So that** I can update information or disable camps

**Acceptance Criteria**:
- Trainer views list of all camps/evaluations
- Can edit form fields, description, pricing
- Can enable/disable camps (turn on/off registration)
- Can delete forms (with confirmation prompt)
- Can view submission count and spots remaining
- Can copy shareable link

**Business Rules**:
- Editing form after submissions: Shows warning "X participants already registered"
- Cannot reduce capacity below current registration count
- Cannot delete form with paid registrations (must refund first via Stripe)
- Disabling camp: Shareable link shows "Registration Closed" message

---

## 8. Data Model

### Form Entity
```
- form_id (UUID, primary key)
- trainer_id (foreign key → trainers table)
- form_type (enum: 'camp', 'evaluation')
- form_name (string, e.g., "Summer Skills Camp")
- description (text)
- price (decimal, nullable, $0 = free)
- capacity_limit (integer, nullable, only for camps)
- is_active (boolean, only for camps)
- form_fields (JSON array of field definitions)
- created_at (timestamp)
- updated_at (timestamp)
- shareable_link (string, unique)
```

### Form Submission Entity
```
- submission_id (UUID, primary key)
- form_id (foreign key → forms table)
- submission_data (JSON, stores all form field responses)
- payment_status (enum: 'free', 'paid', 'pending')
- stripe_payment_id (string, nullable)
- submitted_at (timestamp)
- converted_to_user (boolean, default: false)
- user_id (foreign key → users table, nullable)
```

### Form Field Definition (JSON structure)
```json
{
  "field_id": "unique_id",
  "field_type": "text|email|dropdown|multiselect",
  "label": "Participant Name",
  "required": true,
  "options": ["Option1", "Option2"] // for dropdown/multiselect only
}
```

---

## 9. Business Rules

### Form Creation
1. **Template-Based**: Camps and Evaluations start with pre-loaded templates
2. **Field Types**: Text, Email, Dropdown, Multi-select only
3. **Capacity**: Camps have capacity limits, Evaluations do not
4. **Pricing**: Optional for both camps and evaluations
5. **Availability**: Camps can be turned on/off, Evaluations are always on

### Form Submission
1. **Public Access**: Forms are publicly accessible via shareable link (no login required)
2. **Capacity Enforcement**: If camp is full, form shows "Camp Full" message (no submission allowed)
3. **Payment Required**: For paid camps, payment must complete before registration confirmed
4. **Email Validation**: Email field must be valid format
5. **Duplicate Prevention**: Same email cannot submit to same form twice

### Payment Processing
1. **Stripe Checkout**: All payments processed via Stripe Checkout (no custom payment form)
2. **Application Fee**: Dale's platform cut (5%) applied to camp revenue
3. **Refunds**: Handled manually via Stripe Dashboard for MVP (no automated refund flow)
4. **Payment Confirmation**: Webhook from Stripe confirms payment → Registration marked as "Paid"

### User Conversion
1. **Optional**: Account creation is optional after camp registration
2. **Pre-Filled Data**: Form submission data pre-fills registration form
3. **Auto-Assignment**: Converted users automatically assigned to trainer
4. **Duplicate Check**: If email exists in system, prompt to login instead

### Trainer Permissions
1. **Trainer-Owned**: Trainers can only view/edit their own forms
2. **No Coach Access**: Coaches cannot create or manage forms (trainer-only feature)
3. **Super Admin**: Can view all forms across trainers (impersonation mode)

---

## 10. Integration Points

### Epic-01: User Management & Authentication
- **Camp-to-User Conversion Flow**: After form submission, player can create account with pre-filled data
- **ShareLink Alternative**: If player doesn't convert immediately, send ShareLink for later registration
- **Duplicate Email Check**: Verify email not already in system before allowing account creation

### Epic-05: Payments & Tokens
- **Stripe Checkout Integration**: Redirect to Stripe for payment processing
- **Payment Confirmation Webhook**: Listen for Stripe webhook to mark submission as "Paid"
- **Application Fee**: Dale's cut (5%) applied to camp revenue
- **Refund Handling**: Manual refunds via Stripe Dashboard (no automated flow in MVP)

### Epic-07: Super Admin & System Management
- **Feature Toggle**: "Camps" can be enabled/disabled per trainer
- **Super Admin View**: Can view all forms across trainers (via impersonation)

---

## 11. Technical Notes

### Form Builder
- **Simple Builder**: Add/remove/reorder fields (not drag-and-drop for MVP)
- **Template-Based**: Start with pre-loaded Camps or Evaluations template
- **Field Validation**: Basic client-side validation (required fields, email format)

### Shareable Links
- **Public Access**: No login required to view/submit forms
- **Unique Links**: Each form has unique shareable link (e.g., `app.platform.com/camp/abc123`)
- **SEO Considerations**: Forms should be indexable for organic discovery (optional)

### Payment Flow
1. Player submits form with all fields filled
2. If form has price: Redirect to Stripe Checkout
3. Stripe processes payment
4. Stripe webhook confirms payment
5. Platform marks submission as "Paid" and sends confirmation email
6. Player sees success page with account creation prompt

### Mobile Optimization
- Forms must be mobile-friendly (most submissions from phones)
- Large tap targets for buttons/checkboxes
- Auto-zoom disabled on input fields
- Progress indicators for multi-step payment flow

---

## 12. Open Questions

### Q-08.01: Form Field Limit (P2)
**Question**: What's the maximum number of custom fields per form?  
**Recommendation**: 20 fields max (prevents overly complex forms)  
**Impact**: User experience (long forms = lower completion rates)

### Q-08.02: Form Analytics (P2)
**Question**: Should trainers see completion rates / drop-off points?  
**Recommendation**: Defer to Phase 2 (nice-to-have, not essential for MVP)  
**Impact**: Development time (+1 week if included)

### Q-08.03: Evaluation Time Slot Selection (P2)
**Question**: Should evaluations allow players to select appointment times?  
**Context**: Dale mentioned this as optional enhancement  
**Recommendation**: Defer to Phase 2 (adds significant complexity)  
**Impact**: +2-3 weeks if included (requires calendar integration)

### Q-08.04: Camp Refund Policy (P2)
**Question**: What's the refund policy for camps?  
**Recommendation**: Manual refunds via Stripe for MVP (no automated policy)  
**Impact**: Minimal (trainer handles edge cases manually)

---

## 13. Success Criteria

### Functional Requirements
- ✅ Trainer can create camp form in < 10 minutes
- ✅ Form submission completes in < 3 minutes (including payment)
- ✅ 95%+ payment success rate (Stripe reliability)
- ✅ Shareable links work across all devices
- ✅ Form submissions stored correctly with all data
- ✅ User conversion flow works seamlessly

### Non-Functional Requirements
- ✅ Forms load in < 2 seconds
- ✅ Mobile-friendly (responsive design)
- ✅ Accessible (WCAG 2.1 Level AA compliance)
- ✅ Secure (HTTPS, payment data never stored on platform)

### Business Requirements
- ✅ 70%+ trainer adoption in first 3 months
- ✅ 30%+ conversion rate (camp → full user)
- ✅ Camp revenue accounts for 20%+ of platform transactions
- ✅ Zero payment processing errors

---

## 14. Risks & Mitigations

### Risk: Low Conversion Rate
**Risk**: Camp participants don't convert to full users  
**Mitigation**: 
- Make conversion flow seamless (pre-filled data)
- Send follow-up email with ShareLink
- Offer incentive (e.g., "Free token" for account creation)

### Risk: Payment Processing Failures
**Risk**: Stripe payment fails, player not registered  
**Mitigation**:
- Clear error messages
- Retry payment option
- Support contact for payment issues

### Risk: Form Spam Submissions
**Risk**: Bots submit fake registrations  
**Mitigation**:
- Honeypot fields (hidden field bots fill out)
- Rate limiting (max submissions per IP)
- Email verification (confirm email before marking as valid)

### Risk: Capacity Overselling
**Risk**: Multiple submissions at same time exceed capacity  
**Mitigation**:
- Database-level capacity check (atomic operation)
- Optimistic locking on form submissions

---

## 15. Future Enhancements (Phase 2)

**Advanced Form Features**:
- Conditional logic (show/hide fields based on answers)
- File uploads (medical forms, waivers as PDFs)
- Multi-page forms
- Form analytics and completion tracking
- A/B testing different form versions

**Evaluation Enhancements**:
- Time slot selection for in-person evaluations
- Recurring evaluation requests from players
- Video upload in evaluation forms (skill assessment)

**Camp Enhancements**:
- Early bird pricing (tiered pricing by registration date)
- Group discounts (family/team pricing)
- Waitlist system
- Automated email reminders
- Custom refund policies per camp

**Payment Enhancements**:
- Payment plans (installments for expensive camps)
- Deposit + balance due model
- Scholarships / discount codes for camps

---

**Total User Stories**: 6
**Complexity**: Medium-High (form builder + Stripe integration + conversion flow)
**Priority**: HIGH
