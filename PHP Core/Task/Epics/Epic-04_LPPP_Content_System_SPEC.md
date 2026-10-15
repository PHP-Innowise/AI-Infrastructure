# Epic-04: LP Content System (Learn · Practice)

---

## 1. Description

**Epic-04** implements the **LP Content System (Learn · Practice)**, a modular athlete development system that enables trainers to deliver structured, playlist-driven training content. This epic transforms the platform from a scheduling platform into a comprehensive training ecosystem.

The LP system provides **two pillars** for MVP:
- **LEARN**: Education layer (tutorial videos, step-by-step instruction)
- **PRACTICE**: Execution layer (drills, workouts, repetition)

The **PERFECT** pillar (feedback layer) and **PLAY** pillar (competition layer) are **OUT of MVP** and deferred to Phase 2.

**Implementation Approach**:
- **Learn and Practice are FUNCTIONALLY IDENTICAL**
  - Both are playlists with videos + text content
  - Different labels for content organization purposes only
  - Single implementation with type/category field
  - Players see "Learn" and "Practice" tabs, but backend treats them the same

**Core Features**:
- Playlist-driven content delivery (like YouTube/Spotify playlists)
- YouTube embedded videos (no direct uploads)
- Public/Private content visibility (network effects)
- Trainer-owned content library
- Drill database with public discovery
- Player progress tracking

**Business Value**: LPPP is the platform's **differentiator and upsell feature**. Without LPPP:
- Platform is just a scheduling tool (commoditized)
- No recurring engagement between events
- Limited player development value
- No network effects from shared content
- Trainers cannot scale training beyond in-person events

This epic enables digital training experiences, scalable content delivery, and trainer-to-trainer knowledge sharing.

---

## 2. Business Value

**Problem Statement**: 
Trainers cannot scale their expertise beyond in-person training. Players lack structured learning resources between events. Trainers reinvent the wheel creating drills, and knowledge isn't shared across the platform.

**Success Metrics**:
- ✅ 60%+ of trainers create at least one playlist in first month
- ✅ Average 3-5 playlists per active trainer
- ✅ 50%+ of trainers mark content as PUBLIC (network effects)
- ✅ Average 10 public content items discovered per trainer
- ✅ Player engagement: 40%+ of players complete assigned content
- ✅ Trainer video engagement: Average 5 min watch time per player/week
- ✅ Public drill database grows to 100+ drills in first 3 months

---

## 3. In Scope (MVP)

### LEARN Pillar (Education Layer)
- [ ] Create Learn playlists (video sequences for teaching)
- [ ] Add YouTube videos to playlists (unlisted links)
- [ ] **Add text content to each video** (instructions, key points, explanations)
- [ ] Customize playlists by skill/position/level
- [ ] Mark playlists as Public/Private
- [ ] Assign Learn playlists to players
- [ ] Player view assigned Learn content with text instructions
- [ ] Track player progress (video completion)

### PRACTICE Pillar (Execution Layer)
- [ ] Create drills with metadata (difficulty, equipment, duration)
- [ ] **Add text content to each drill** (setup instructions, coaching points, variations)
- [ ] Drill database (trainer's own drills)
- [ ] Public drill discovery (search/filter public drills)
- [ ] Create Practice playlists (drill sequences/workouts)
- [ ] Assign Practice playlists to players
- [ ] Player view assigned Practice content with text instructions
- [ ] Track player progress (drill completion)

### Content Management (Cross-Pillar)
- [ ] Content visibility toggle (Public/Private per item)
- [ ] **Coach-Only Content**
  - Trainers can create playlists visible only to coaches (not players)
  - Use cases: Coach certifications, training methodology, internal videos
  - Simple visibility toggle in playlist settings
- [ ] Content attribution (track original creator)
- [ ] Content discovery (search public content)
- [ ] Content reuse (add others' public content to own playlists)

### Player Experience
- [ ] LP portal (Learn, Practice tabs)
- [ ] **View content library with paywall** (see all content, pay for access)
- [ ] **Trainer content suggestions** (highlighted recommended content)
- [ ] **Separated views for multi-trainer players** (content per trainer context)
- [ ] Purchase content access (integration with Epic-05)
- [ ] Mark content as complete
- [ ] Progress dashboard (completion stats)

---

## 4. Out of Scope (Post-MVP)

**Phase 2 Deferrals**:
- ❌ **PERFECT pillar (Feedback Layer)** - Player submits video link, trainer provides feedback - removed from MVP
- ❌ PLAY pillar (competition layer)
- ❌ Direct video uploads (platform uses external video hosting links only)
- ❌ Video feedback (trainer response videos)
- ❌ AI-powered feedback analysis
- ❌ Video annotations and drawing tools
- ❌ Player-facing content attribution ("Drill by Coach Sarah")
- ❌ Usage analytics for public content creators
- ❌ Content licensing or revenue sharing
- ❌ Workout templates (pre-built sequences)
- ❌ Scheduled content release (content available immediately)
- ❌ Content recommendations (AI-suggested playlists)
- ❌ Player video comparison (side-by-side before/after)
- ❌ Leaderboards (player rankings by content completion)
- ❌ Badges and achievements
- ❌ Social features (player comments, likes, shares)
- ❌ Content versioning (edit history, rollback)
- ❌ Content export (download playlists)
- ❌ Multi-language content
- ❌ Content moderation (flagging inappropriate content)

**Simplifications Applied**:
- ✅ **Learn + Practice consolidated** (same functionality, different labels only)
- ✅ Advanced content organization removed
- ✅ **Player progress tracking simplified** (auto-complete when player starts watching, no manual button)
- ✅ Public library curation removed (light-touch only)
- ✅ Content engagement analytics removed

---

## 5. Dependencies

### Required Before This Epic
- **Epic-01** (User Management) - Players, trainers must exist
- **Epic-02** (Event Management) - Can assign content to events

### Blocks These Epics
- None (LPPP is parallel to core CRM/events)

### External Dependencies
- **YouTube Platform**: For video hosting, embedding, player
- **YouTube API** (Optional MVP): For video metadata (duration, thumbnail)
- **File Storage** (Optional): For drill thumbnails, trainer avatars

---

## 6. User Roles Involved

| Role | Interaction | Permissions |
|:---|:---|:---|
| **Trainer / Business Owner** | Creates content, playlists, assigns to players, provides feedback | Full control over own content, can discover/use public content from others |
| **Coach / Contractor** | Views assigned players' progress, can view trainer's content | **Cannot create content** (only trainers can create), can view progress of players in their assigned events |
| **Player / Parent** | Views assigned content, tracks progress, submits feedback requests | Can only view assigned content, cannot create content |
| **Super Admin** | Views all content, can mark content as featured, analytics | Can view all content across trainers, cannot edit trainer content |

---

## 7. User Stories & Acceptance Criteria

### US-04.01: Trainer Creates Learn Playlist

**As a** Trainer  
**I want to** create a Learn playlist with tutorial videos  
**So that** players can learn fundamentals at their own pace

**Acceptance Criteria**:
- [ ] Navigate to "LPPP" → "Learn" tab
- [ ] Click "+ Create Learn Playlist"
- [ ] Modal/page opens: Create Learn Playlist
- [ ] Fields:
  - Playlist title (required, max 100 characters)
  - Description (optional, text area)
  - Customization filters (optional):
    - Skill level (multi-select: Beginner, Intermediate, Advanced, Elite)
    - Position (multi-select: Guard, Forward, Center, etc. - sport-specific)
    - Age level (multi-select: Youth, Teen, Adult)
  - Visibility toggle: **Private** (default) or **Public**
- [ ] Click "Add Videos"
- [ ] For each video:
  - Paste YouTube URL (unlisted link)
  - Video title (required)
  - **Text Content / Instructions** (optional, rich text area)
    - Setup instructions (e.g., "Watch this video in a quiet place with a notebook")
    - Key points to focus on (e.g., "Pay attention to foot positioning at 2:15")
    - Additional explanations or context
    - Post-video exercises or reflection questions
  - Tags (optional, comma-separated: "fundamentals", "beginner", "shooting")
  - Duration (auto-detected from YouTube API if available, or manual input)
- [ ] Drag-and-drop to reorder videos in playlist
- [ ] Save playlist → Playlist created
- [ ] Playlist appears in Learn tab list
- [ ] Confirmation: "Learn playlist created!"

**Validation**:
- [ ] Playlist title required (non-empty)
- [ ] At least 1 video required
- [ ] YouTube URLs validated (format check)
- [ ] Duplicate videos allowed (same video in different order)

**Use Cases**:
- "Fundamentals of Shooting" playlist (5 videos)
- "Footwork Drills Series" playlist (8 videos)
- "Basketball IQ - Game Awareness" playlist (3 videos)

---

### US-04.02: Trainer Creates Drill in Practice Pillar

**As a** Trainer  
**I want to** create a drill with metadata  
**So that** I can organize drills and share them publicly

**Acceptance Criteria**:
- [ ] Navigate to "LPPP" → "Practice" → "Drill Database"
- [ ] Click "+ Create New Drill"
- [ ] Form opens: Create Drill
- [ ] Fields:
  - Drill name (required, max 100 characters)
  - **Text Content / Instructions** (optional, rich text area up to 1000 characters)
    - Drill setup instructions (e.g., "Place 5 cones in a line, 2 feet apart")
    - Execution steps (e.g., "1. Dribble with right hand through cones, 2. Switch to left hand on return")
    - Coaching points (e.g., "Keep head up, eyes forward")
    - Common mistakes to avoid
    - Variations for different skill levels
  - YouTube URL (required, unlisted link showing drill demonstration)
  - Difficulty level (dropdown: Beginner, Intermediate, Advanced, Elite)
  - Equipment needed (multi-select tags: Ball, Cones, Ladder, Hoop, None, etc.)
  - Space requirement (dropdown: Small (10x10), Medium (half-court), Large (full court))
  - Player count (text: "1", "1-2", "2-4", "5+")
  - Duration (min/max in minutes, e.g., 5-10 minutes)
  - Categories (multi-select checkboxes: Dribbling, Shooting, Passing, Defense, Footwork, Conditioning, etc.)
  - Visibility toggle: **Private** (default) or **Public**
- [ ] Save → Drill created
- [ ] Drill appears in "My Drills" section of Drill Database
- [ ] If Public: Drill appears in public drill discovery for all trainers
- [ ] Confirmation: "Drill created!"

**Validation**:
- [ ] Drill name required
- [ ] YouTube URL required and valid format
- [ ] At least 1 category required

**Use Cases**:
- "Cone Dribbling Series" drill (Public, Beginner, Equipment: Ball + Cones)
- "3-Point Shooting Circuit" drill (Private, Advanced, Equipment: Ball + Hoop)
- "Defensive Slide Drill" drill (Public, Intermediate, Equipment: None)

---

### US-04.03: Trainer Discovers and Uses Public Drill

**As a** Trainer  
**I want to** search and discover public drills from other trainers  
**So that** I can use proven drills without reinventing the wheel

**Acceptance Criteria**:
- [ ] Navigate to "LPPP" → "Practice" → "Drill Database"
- [ ] Toggle: "My Drills" / "Public Drills"
- [ ] Select "Public Drills"
- [ ] Search bar: "Search drills..." (searches drill name, description, tags)
- [ ] Filters:
  - Category (checkboxes: Dribbling, Shooting, etc.)
  - Difficulty level (checkboxes)
  - Equipment needed (checkboxes)
  - Duration range (slider: 0-60 minutes)
- [ ] Drill list displays matching public drills
- [ ] Each drill card shows:
  - Drill name
  - Creator (trainer name): "By Coach Sarah"
  - Video thumbnail (from YouTube)
  - Difficulty badge, duration, equipment badges
  - Description (truncated)
  - "Preview" button, "Add to Playlist" button
- [ ] Click "Preview" → Modal opens with:
  - Full drill details
  - Embedded YouTube video
  - Equipment, duration, categories
  - "Add to Playlist" button
- [ ] Click "Add to Playlist":
  - Select existing Practice playlist OR create new playlist
  - Drill added to selected playlist
  - Original creator reference stored (attribution)
- [ ] Confirmation: "Drill added to [Playlist Name]"

**Search/Filter Logic**:
- [ ] Search matches drill name, description, tags
- [ ] Filters combine with AND logic
- [ ] Empty results: "No drills match your filters. Try adjusting criteria."

**Attribution**:
- [ ] Original creator tracked in database
- [ ] Trainer sees creator name in drill list (admin view)
- [ ] Player-facing attribution: Defer to Phase 2

---

### US-04.04: Trainer Creates Practice Playlist (Workout)

**As a** Trainer  
**I want to** create a Practice playlist with multiple drills  
**So that** players can follow a structured workout sequence

**Acceptance Criteria**:
- [ ] Navigate to "LPPP" → "Practice" tab
- [ ] Click "+ Create Practice Playlist"
- [ ] Form opens: Create Practice Playlist
- [ ] Fields:
  - Playlist title (required)
  - Description (optional)
  - Customization filters (optional): Skill, position, age
  - Visibility toggle: Private/Public
- [ ] Click "Add Drills"
- [ ] Options:
  - **Option A**: "Add from Drill Database" → Opens drill picker modal
    - Search/filter drills (my drills + public drills)
    - Select drill(s), click "Add"
  - **Option B**: "Add New Drill" → Create drill inline (see US-04.02)
- [ ] Drills added to playlist
- [ ] Drag-and-drop to reorder drills
- [ ] For each drill, optionally add:
  - Trainer notes (e.g., "Focus on form, 3 sets of 10 reps")
  - Estimated time (if different from drill default)
- [ ] Remove drill button (X icon)
- [ ] Save playlist → Practice playlist created
- [ ] Playlist appears in Practice tab list

**Workout Example**:
- "30-Minute Shooting Workout"
  1. "Form Shooting" drill (5 min)
  2. "Catch and Shoot" drill (10 min)
  3. "3-Point Circuit" drill (10 min)
  4. "Free Throw Challenge" drill (5 min)

---

### US-04.05: Trainer Assigns Playlist to Players

**As a** Trainer  
**I want to** assign playlists to specific players or groups  
**So that** they receive structured training content

**Acceptance Criteria**:
- [ ] From playlist (Learn or Practice), click "Assign to Players"
- [ ] Modal opens: Assign Playlist
- [ ] Select players:
  - Individual player selection (multi-select list)
  - OR Select by Label (e.g., "Beginners" label from Epic-03)
  - OR Select by skill level/age (filter players)
  - Show player count: "15 players selected"
- [ ] Optional: Set due date (date picker)
- [ ] Optional: Add assignment note (visible to players)
- [ ] Click "Assign" → Playlist assigned
- [ ] Players receive notification (email + in-app):
  - "New content assigned: [Playlist Name]"
  - Link to LPPP portal
- [ ] Playlist appears in players' LPPP portal
- [ ] Assignment tracked (who assigned, when, to whom)

**Assignment Status**:
- [ ] Trainer can view: "Assigned to 15 players, 8 completed, 7 in progress"
- [ ] Click status → View individual player progress

**Reassignment**:
- [ ] Trainer can assign same playlist to different players anytime
- [ ] No limit on assignments

---

### US-04.06: Trainer Marks Content as Public

**As a** Trainer  
**I want to** mark my playlists or drills as Public  
**So that** other trainers can discover and use my content

**Acceptance Criteria**:
- [ ] From playlist list or drill database, find own content
- [ ] Toggle "Visibility": **Private** or **Public**
- [ ] If changing to Public:
  - Confirmation: "Make [Content] public? Other trainers will be able to discover and use it."
  - Trainer retains ownership (can change back to Private anytime)
- [ ] Content marked Public → Appears in public discovery for all trainers
- [ ] Badge shown: "Public" (green badge)

**Content Attribution** (per Q-P1-016):
- [ ] Original creator stored in database
- [ ] Other trainers see: "By [Trainer Name]" in admin views
- [ ] If creator changes to Private later:
  - Content removed from public discovery
  - Existing playlist references keep working (don't break)

**Use Cases**:
- Trainer creates amazing footwork drill → Marks Public → Other trainers discover and use
- Trainer creates beginner shooting playlist → Marks Public → Platform library grows
- Network effects: Shared knowledge benefits all trainers

---

### US-04.07: Player Views Assigned Content (LPPP Portal)

**As a** Player or Parent  
**I want to** view and access training content from my trainer  
**So that** I can learn and practice between events

**Acceptance Criteria**:
- [ ] Navigate to "LPPP" (main menu)
- [ ] **Multi-Trainer Context**: Content shown for currently selected trainer only
- [ ] LPPP portal opens with tabs: **Learn**, **Practice**, **Perfect**, **Progress**

**Learn Tab**:
- [ ] **Content Library** section shows all available Learn playlists:
  - Shows both purchased and unpurchased playlists
  - **Locked indicator** (🔒) for unpaid content
  - **Suggested badge** for trainer-recommended content
- [ ] Each playlist shows:
  - Title, description
  - Trainer name
  - Price/purchase status (if paywall enabled): "Free", "$20", "Subscribed"
  - Locked/Unlocked status
  - Progress (if purchased): "3 of 5 videos completed"
- [ ] Click **locked playlist** → Purchase/payment flow (Epic-05)
- [ ] Click **unlocked/purchased playlist** → Opens playlist detail:
  - List of videos in order
  - Each video shows: Title, duration, completion checkmark
  - Click video → Opens video player page

**Practice Tab**:
- [ ] List of assigned Practice playlists (workouts)
- [ ] Each playlist shows:
  - Title, description
  - Assigned by, due date
  - Progress: "2 of 4 drills completed"
- [ ] Click playlist → Opens workout detail:
  - List of drills in order
  - Each drill shows: Title, duration, equipment, completion checkmark
  - Trainer notes (if any)
  - Click drill → Opens drill player page

**Video/Drill Player Page**:
- [ ] Embedded YouTube player (unlisted video)
- [ ] Video title
- [ ] **Text Instructions Section** (displayed prominently below or beside video):
  - Setup instructions, key points, coaching notes
  - Formatted text with bullet points, numbered lists
  - Expandable/collapsible if long
- [ ] Playback controls (play, pause, seek, volume)
- [ ] **Auto-completion**: When player clicks play → Content automatically marked complete
- [ ] Completion checkmark appears (✓ Completed)
- [ ] "Next" button (go to next video/drill in playlist)

**Parent Behavior** (Child Account):
- [ ] Parent can switch between children (dropdown)
- [ ] View assigned content per child
- [ ] Auto-completion works same way (when play button clicked)

---

### US-04.08: Player Tracks Progress

**As a** Player or Parent  
**I want to** see my training progress  
**So that** I stay motivated and see what I've completed

**Acceptance Criteria**:
- [ ] Navigate to "LPPP" → "Progress" tab
- [ ] Dashboard shows:

**Overall Stats**:
- [ ] Total playlists assigned: X
- [ ] Total videos/drills completed: Y
- [ ] Total watch time: Z hours
- [ ] Completion rate: X% (completed / total assigned)

**Recent Activity**:
- [ ] List of recently completed content:
  - Content title, pillar (Learn/Practice), completion date
  - "Completed [Video Name] on [Date]"

**Incomplete Assignments**:
- [ ] List of assigned playlists not yet completed
- [ ] Sorted by due date (nearest first)
- [ ] Click → Opens playlist

**Progress per Playlist**:
- [ ] For each assigned playlist:
  - Progress bar: 60% complete
  - "3 of 5 items completed"

**Trainer View of Player Progress**:
- [ ] Trainer can view player progress from CRM (Epic-03 player detail)
- [ ] Shows: Assigned playlists, completion status, last activity

---

### US-04.09: Trainer Edits Playlist

**As a** Trainer  
**I want to** edit existing playlists  
**So that** I can update content and keep it current

**Acceptance Criteria**:
- [ ] From playlist list (Learn or Practice), click "Edit"
- [ ] Edit form opens with current playlist data pre-filled
- [ ] Editable fields:
  - Title, description
  - Customization filters (skill, position, age)
  - Visibility (Public/Private toggle)
  - Video/drill order (drag-and-drop to reorder)
  - Add new videos/drills
  - Remove videos/drills (X button)
- [ ] Save → Playlist updated
- [ ] If playlist already assigned to players:
  - Players see updated content immediately
  - Progress not affected (completed items stay completed)
  - New items appear as "not started"
- [ ] Confirmation: "Playlist updated!"

**Edge Cases**:
- [ ] If video removed: Player progress for that video remains (for history)
- [ ] If video reordered: Player progress unaffected

---

### US-04.10: Trainer Deletes Playlist or Drill

**As a** Trainer  
**I want to** delete playlists or drills  
**So that** I can clean up unused content

**Acceptance Criteria - Delete Playlist**:
- [ ] From playlist list, click "Delete" (trash icon)
- [ ] Confirmation: "Delete [Playlist Name]? This will unassign it from all players."
- [ ] Confirm → Playlist deleted
- [ ] Playlist removed from assigned players' LPPP portal
- [ ] Player progress history preserved (for trainer analytics)
- [ ] If Public: Removed from public discovery
- [ ] If other trainers using it: Their references break (show "Content unavailable")

**Acceptance Criteria - Delete Drill**:
- [ ] From drill database, click "Delete" (trash icon)
- [ ] If drill is in playlists:
  - Warning: "This drill is used in [N] playlists. Delete anyway?"
  - If deleted: Drill removed from playlists
- [ ] Confirm → Drill deleted
- [ ] If Public: Removed from public discovery
- [ ] If other trainers using it: Their playlist references show "Drill unavailable"

**Soft Delete Recommendation** (Technical):
- [ ] Don't hard-delete from database (soft delete for data integrity)
- [ ] Mark as deleted, hide from UI
- [ ] Preserve for analytics and history

---

### US-04.11: Super Admin Views LPPP Analytics

**As a** Super Admin  
**I want to** view system-wide LPPP usage analytics  
**So that** I can understand platform engagement

**Acceptance Criteria**:
- [ ] Navigate to "Super Admin" → "LPPP Analytics"
- [ ] Dashboard shows:

**Content Stats**:
- [ ] Total playlists created (all trainers)
- [ ] Total drills created
- [ ] Public vs Private ratio
- [ ] Most used public drills (top 10)
- [ ] Top content creators (trainers with most public content)

**Engagement Stats**:
- [ ] Total player video views (all time, this week)
- [ ] Average watch time per player
- [ ] Completion rate (% of assigned content completed)
- [ ] Feedback requests submitted (total, this week)
- [ ] Feedback requests completed (response rate)

**Growth Trends**:
- [ ] Content creation trend (playlists/drills per week)
- [ ] Player engagement trend (views per week)
- [ ] Public content growth (new public items per week)

**Drill-Down**:
- [ ] Select specific trainer → View trainer-specific LPPP stats
- [ ] Export data to CSV (for reporting)

---

## 8. Data Requirements

### What Information Needs to Be Stored

**For Playlists**:
- Playlist unique identifier
- Pillar (Learn, Practice)
- Trainer who created it
- Title and description
- Customization filters (skill, position, age levels)
- Visibility (Public or Private)
- Created and updated timestamps
- If public: Public date

**For Content Items** (Videos, Drills):
- Content unique identifier
- Trainer who created it (original creator)
- Pillar (Learn, Practice, Perfect)
- Type (video, drill, feedback)
- Title (short, max 100 chars)
- **Text Instructions / Content** (rich text, up to 1000 chars)
  - Setup instructions, key points, coaching notes
  - Stored as formatted text (supports bullet points, numbered lists)
- YouTube URL (unlisted link)
- Duration (seconds)
- Visibility (Public or Private)
- Tags (array of strings)
- Created and updated timestamps

**For Drills** (Practice Pillar):
- Content reference (inherits from Content)
- Difficulty level (Beginner, Intermediate, Advanced, Elite)
- Equipment needed (array: Ball, Cones, Ladder, etc.)
- Space requirement (Small, Medium, Large)
- Player count (text: "1-2", "2-4", etc.)
- Duration range (min/max minutes)
- Categories (array: Dribbling, Shooting, Passing, etc.)

**For Playlist-Content Association** (Many-to-Many):
- Which playlist
- Which content item
- Sequence order (position in playlist)
- Is required (boolean - optional)
- Trainer notes (specific to this playlist context)

**For Content Assignments**:
- Which playlist
- Which player(s)
- Assigned by (trainer or coach)
- Assigned timestamp
- Due date (optional)
- Assignment note (optional, visible to player)
- Status (not started, in progress, completed)

**For Content Progress**:
- Which content item
- Which player
- Status (not started, in progress, completed)
- Progress percentage (0-100, for video watch progress)
- Completed timestamp
- Watch time (seconds spent on content)

**For Content Attribution** (Public Content):
- Original creator (trainer reference)
- Trainers using this content (array for analytics - optional)
- Usage count (how many trainers added to playlists)

---

## 9. Business Rules & Logic

### YouTube Video Strategy

**Video Hosting**:
- All videos hosted on YouTube (external)
- Dale and trainers upload videos to YouTube separately
- Videos set to **Unlisted** (accessible via link only, not publicly searchable)
- Platform stores YouTube URLs only (no video files stored)

**Embedding**:
- Videos displayed in embedded YouTube player
- Player features: Play, pause, seek, volume, fullscreen
- YouTube handles streaming, transcoding, CDN, adaptive quality

**Benefits**:
- No video storage costs
- No transcoding infrastructure needed
- Leverages YouTube's reliability and performance
- Trainers already familiar with YouTube

### Content Visibility Rules

**Private Content**:
- Visible only to creator (trainer)
- Creator can assign to own players
- Not discoverable by other trainers
- Default visibility for new content

**Public Content**:
- Visible to all trainers in public discovery (search/filter)
- Any trainer can add to their own playlists
- Original creator retains ownership
- Creator can change back to Private anytime

**Public to Private Transition**:
- Content removed from public discovery
- Other trainers' playlist references **keep working** (don't break)
- Ensures content continuity for players

### Content Monetization Rules (Paywall Model - D-SCOPE-011)

**Paywall Model**:
- Players can **see content library** (playlists, videos) but **cannot access without payment**
- Trainer can **suggest specific content** to players (recommendation system)
- Players must **purchase access** to view content
- Payment processed through Epic-05 (trainer-specific tokens, subscriptions, or one-time)

**Access Control**:
- Unpaid content shown with "locked" indicator
- Click locked content → Payment/purchase flow
- After purchase: Full access to content
- Access persists (one-time purchase model or subscription-based)

**Pricing Structure** (To be finalized with client):
- **Option A**: Per-playlist pricing (one-time purchase)
- **Option B**: Subscription model (monthly access to all content)
- **Option C**: Bundle pricing (multiple playlists at discount)
- **Option D**: Included with event subscription (unlimited training + content)

**Trainer Content Suggestions**:
- Trainer can "suggest" specific playlists to specific players
- Suggestion appears as notification + highlighted in player's LPPP portal
- Player still needs to purchase to access (if paywall enabled for that content)
- Free content can be suggested without purchase requirement

### Multi-Trainer Content Visibility

**Separated Views for Players with Multiple Trainers**:
- Players see content library for **current trainer context only**
- Switch trainer context → Different content library visible
- Each trainer's content completely isolated
- Player can purchase content from multiple trainers (separate purchases per trainer)
- Aligns with separated views architecture (events, tokens, content all isolated)

### Content Attribution Rules

**Original Creator Tracking**:
- Every piece of content has original creator stored
- Creator name shown in admin views ("By Coach Sarah")
- Player-facing attribution: Defer to Phase 2

**Content Reuse**:
- Trainer B can add Trainer A's public drill to their playlist
- Drill reference stored (not copied)
- If Trainer A deletes drill: Trainer B's reference breaks (show "Unavailable")

**Usage Analytics** (Defer to Phase 2):
- Track how many trainers use each public content item
- Track total views per public content
- Show original creator their content's reach

### Playlist Assignment Rules

**Assignment Targets**:
- Can assign to individual players
- Can assign to entire label group (e.g., "Beginners")
- Can assign to filtered players (e.g., all players with skill level = Advanced)

**Multiple Assignments**:
- Same playlist can be assigned to different players/groups
- Each assignment tracked separately

**Reassignment**:
- Can reassign same playlist to same player (refreshes assignment)
- **Player progress kept** (existing progress preserved, not reset)

### Progress Tracking Rules

**Video/Drill Completion**:
- **Auto-complete when player starts watching/viewing** → Content marked complete
- Triggered when player clicks play or opens content (engagement started)
- Timestamp recorded
- Progress bar updates (e.g., 3 of 5 completed = 60%)

**Completion Criteria**:
- Automatic: Content marked complete when player starts watching (clicks play button)
- No manual "Mark as Complete" button needed
- Simple engagement tracking

**Progress Persistence**:
- If content removed from playlist: Progress preserved (for history)
- If playlist deleted: Progress preserved (trainer can view in analytics)
- If playlist reassigned to same player: **Existing progress kept** (not reset)

### Drill Database Rules

**Drill Creation**:
- Trainers create drills with metadata
- Drills stored in personal drill database
- Default visibility: Private

**Public Drill Discovery**:
- Trainers can search/filter public drills
- Filters: Category, difficulty, equipment, duration
- Preview drill before adding to playlist

**Drill Reuse**:
- Add public drill to own Practice playlist
- Drill reference stored (not copied)
- If creator deletes: Reference breaks

---

## 10. User Flows

### Flow 1: Trainer Creates and Assigns Learn Playlist

1. Trainer logs in to LPPP section
2. Navigate to "Learn" tab
3. Click "+ Create Learn Playlist"
4. Enter playlist details:
   - Title: "Shooting Fundamentals"
   - Description: "Learn proper shooting form from basics to advanced"
   - Skill level: Beginner, Intermediate
   - Visibility: Private
5. Click "Add Videos"
6. Add 5 videos:
   - Video 1: "Proper Stance and Grip" (YouTube URL)
   - Video 2: "Elbow Alignment" (YouTube URL)
   - Video 3: "Follow-Through Technique" (YouTube URL)
   - Video 4: "Common Mistakes" (YouTube URL)
   - Video 5: "Practice Routine" (YouTube URL)
7. Drag-and-drop to reorder if needed
8. Save playlist → "Shooting Fundamentals" playlist created
9. Click "Assign to Players"
10. Select players: All players with "Beginner" label (12 players)
11. Set due date: 2 weeks from today
12. Add note: "Complete this before next practice event"
13. Click "Assign" → 12 players assigned
14. Players receive notification: "New content assigned: Shooting Fundamentals"
15. Players see playlist in their LPPP portal

### Flow 2: Trainer Discovers Public Drill and Adds to Workout

1. Trainer creating new Practice playlist: "Advanced Dribbling Workout"
2. Click "Add from Drill Database"
3. Toggle to "Public Drills"
4. Search: "dribbling"
5. Filter: Difficulty = Advanced, Equipment = Ball + Cones
6. Results show 8 public drills
7. See drill: "Cone Weave Speed Dribble" by Coach Sarah
8. Click "Preview" → Watch drill video, review details
9. Click "Add to Playlist" → Drill added to "Advanced Dribbling Workout"
10. Add 3 more drills (mix of own drills and public drills)
11. Reorder drills into logical sequence
12. Save playlist → Workout created
13. Assign to "Elite Squad" label (8 players)
14. Players receive workout, can follow drill sequence

### Flow 3: Player Completes Learn Playlist and Tracks Progress

1. Player (Sarah) logs in to LPPP portal
2. Navigate to "Learn" tab
3. See assigned playlist: "Shooting Fundamentals" (Due in 2 weeks)
4. Click playlist → Opens playlist detail
5. See 5 videos, none completed yet (0 of 5)
6. Click Video 1: "Proper Stance and Grip"
7. Video player opens, embedded YouTube video
8. Click play button → **Auto-complete triggered** → Video 1 marked complete
9. Progress updates: 1 of 5 (20%)
10. Watch video (3 minutes), read text instructions
11. Click "Next" → Video 2 opens
12. Click play on Video 2 → Auto-marked complete
13. Progress: 2 of 5 (40%)
14. Sarah completes all 5 videos over next few days (clicks play on each)
15. Progress: 5 of 5 (100% complete)
16. Trainer receives notification: "Sarah completed Shooting Fundamentals"
17. Navigate to "Progress" tab → Dashboard shows:
    - 1 playlist completed
    - Total watch time: 18 minutes
    - Completion rate: 100%

### Flow 4: Trainer Marks Drill as Public, Other Trainers Discover

1. Trainer A (Coach Sarah) has created amazing drill: "Defensive Slide Sequence"
2. Navigate to Drill Database → "My Drills"
3. Find "Defensive Slide Sequence" drill
4. Currently Private
5. Toggle Visibility: **Public**
6. Confirmation: "Make drill public? Other trainers will be able to use it."
7. Confirm → Drill now Public
8. Badge shows: "Public" (green)

**Other Trainer Discovers**:
9. Trainer B (Coach Mike) creating defensive workout
10. Navigate to Drill Database → Public Drills
11. Search: "defensive"
12. Filter: Category = Defense
13. Results show "Defensive Slide Sequence" by Coach Sarah
14. Preview drill → Looks great!
15. Click "Add to Playlist" → Adds to "Defensive Fundamentals" workout
16. Assign workout to players
17. Players complete drill, benefit from Coach Sarah's expertise
18. Coach Sarah's drill now helps multiple trainers' players

**Network Effects**:
- More trainers create public content
- Public drill database grows
- All trainers benefit from shared knowledge
- Players receive best practices from across platform

---

## 11. Performance & Scale Targets

**Response Times**:
- LP portal load: <2 seconds
- Playlist detail load: <1 second
- Video player load: <2 seconds (depends on YouTube)
- Drill database search: <1 second

**Note**: Specific technical implementation (caching, pagination, YouTube API) decided by developer.

---

## 12. Questions / Open Issues

| ID | Question | Priority | Status | Owner |
|:---|:---|:---:|:---|:---|
| Q-04.03 | Content moderation: How to handle inappropriate public content? Flag system? | P2 | Open | Team |
| Q-04.06 | Playlist due dates: Reminder notifications 24 hours before? | P2 | Open | Client |
| Q-04.09 | Can trainers schedule playlist assignments (release on specific date)? | P2 | Open | Client |
| Q-04.10 | Content versioning: If trainer edits public content, notify trainers using it? | P2 | Open | Team |

---

## 13. Acceptance Criteria (Epic-Level)

This Epic is complete when:

**LEARN Pillar**:
- [ ] Trainer can create Learn playlists with YouTube videos
- [ ] **Trainer can add text instructions/content to each video** (setup, key points, explanations)
- [ ] Trainer can customize playlists by skill/position/age
- [ ] Trainer can mark playlists as Public or Private
- [ ] Trainer can assign playlists to players
- [ ] **Player can view assigned Learn content with text instructions**
- [ ] **Videos auto-complete when player starts watching** (clicks play)
- [ ] Player progress tracked correctly

**PRACTICE Pillar**:
- [ ] Trainer can create drills with metadata (difficulty, equipment, duration, categories)
- [ ] **Trainer can add text instructions to each drill** (setup, steps, coaching points)
- [ ] Drill database stores trainer's drills
- [ ] Trainer can search/filter public drills
- [ ] Trainer can discover and add public drills to own playlists
- [ ] Trainer can create Practice playlists (workout sequences)
- [ ] Player can view assigned Practice content
- [ ] **Drills auto-complete when player starts viewing** (clicks play)

**Content Management**:
- [ ] Content visibility toggles work (Public/Private)
- [ ] Public content discoverable by all trainers
- [ ] Content attribution tracks original creator
- [ ] Content reuse (add others' public content) works correctly
- [ ] Public to Private transition doesn't break existing references

**Player Experience**:
- [ ] LP portal accessible (Learn, Practice, Progress tabs)
- [ ] Assigned content displays correctly
- [ ] Video/drill player works (YouTube embedding)
- [ ] Progress tracking accurate (completion, watch time)
- [ ] Progress dashboard shows stats correctly

**Data Integrity**:
- [ ] Playlists and content stored correctly
- [ ] Progress records persist (even if content deleted)
- [ ] Attribution preserved (original creator tracked)
- [ ] Assignment tracking works (who assigned, when, to whom)

**Performance**:
- [ ] LP portal loads in <2 seconds
- [ ] Drill database search in <1 second
- [ ] YouTube videos embed and play smoothly

**Approval**:
- [ ] Demo approved
- [ ] All P0 questions resolved
- [ ] Public content network effects validated
- [ ] YouTube embedding strategy confirmed working

---

## 15. Mockups / Design References

**Key Screens to Design**:
1. LP Dashboard (trainer) - Pillar tabs (Learn, Practice)
2. Create Learn Playlist form
3. Create Drill form (Practice pillar)
4. Drill Database view (My Drills / Public Drills)
5. Public drill discovery (search/filter interface)
6. Practice Playlist builder (workout sequence)
7. Assign Playlist to Players modal
8. LP Portal (player) - Learn, Practice, Progress tabs
9. Video/Drill player page (embedded YouTube)
10. Progress Dashboard (player stats)

---

## 16. Testing Considerations

**What Should Be Tested**:

**Functional Testing**:
- All user stories and acceptance criteria met
- Both pillars work correctly (Learn, Practice)
- Public/Private content visibility works
- Progress tracking accurate

**Key Scenarios to Test**:
- Trainer creates Learn playlist, assigns to players, players complete content
- Trainer creates drill, marks Public, other trainer discovers and uses
- Trainer toggles content Public → Private, existing references still work
- **Player clicks play on videos, auto-completes, progress dashboard updates correctly**
- **Playlist reassigned to same player, existing progress preserved**
- Trainer edits playlist, assigned players see updated content
- Trainer deletes playlist, players no longer see it

**Edge Cases**:
- YouTube URL invalid (validation)
- Video player fails to load (YouTube API issues)
- **Player clicks play multiple times, completion remains idempotent**
- **Playlist reassigned to player who already has progress, progress kept not reset**
- Trainer deletes drill used in other trainers' playlists (reference breaks gracefully)
- Player completes playlist after due date (still recorded)
- Drill database search returns 0 results (empty state)

**Security Testing**:
- Player cannot access unassigned content
- Trainer cannot edit others' content (only own content)
- YouTube URLs validated (no malicious links)
- Public content discoverable by all, Private content truly private

**Performance Testing**:
- LP portal load with 100 playlists
- Drill database search with 1,000+ public drills
- Player progress calculation with 500+ content items
- YouTube video embedding with 50 concurrent players

**YouTube Integration Testing**:
- Unlisted videos embed correctly
- Video playback controls work
- Video thumbnails display
- Duration auto-detection from YouTube API (if implemented)
- Handling YouTube API rate limits

**Note**: Developer chooses specific testing tools and frameworks.

---

## 17. Implementation Notes

**Suggested Implementation Order**:
1. Basic data models (Playlist, Content, Drill)
2. Learn pillar - Create playlists, add videos
3. Assign playlists to players
4. Player LPPP portal - View assigned content
5. YouTube video embedding
6. Progress tracking (mark complete, stats)
7. Practice pillar - Drill database, create drills
8. Public drill discovery and content reuse
9. Practice playlists (workout sequences)
10. Content visibility management (Public/Private toggle)
11. Super Admin LP analytics

**Integration Points with Other Epics**:
- **Epic-01** (User Management): Player profiles, trainer accounts
- **Epic-02** (Event Management): Can assign playlists to specific events
- **Epic-03** (CRM): Player progress visible in player detail, can assign to label groups
- **Epic-05** (Payments): Content purchase integration

**YouTube API Integration**:
- YouTube Data API v3 for metadata (duration, thumbnail)
- YouTube IFrame Player API for embedding
- API key required (set up in project config)
- Rate limits: 10,000 units/day (sufficient for MVP)

**Security Requirements** (High-Level):
- Validate YouTube URLs (format check, prevent XSS)
- Sanitize user input (playlist titles, drill descriptions)
- Ensure content visibility rules enforced (Private not leaked)
- Validate assignment permissions (trainer can only assign own content)

**Notification Requirements**:
- Player notified when playlist assigned
- Optional: Reminder notifications for due dates

*Note: Specific email service and notification system chosen by developer*

---

**Priority**: P1 - Platform Differentiator (upsell feature, competitive advantage)
**Complexity**: High (YouTube integration, two distinct pillars, public content network, progress tracking)

**Note**: This specification focuses on **business requirements** and **user needs**. Technical implementation details (database schema, YouTube API integration, caching strategies, specific technologies) are decided by the development team based on their expertise.

