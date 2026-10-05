# Coaching policy — rationale and sources

Policy version **2026.10-draft.1** (`crates/domain/src/policy.rs`). Every plan and proposal records the policy version it was validated against.

> **Review status: draft.** The limits below were chosen conservatively by the implementer from the sources listed. They have **not** been reviewed by a qualified coach or clinician; that review is an open release gate. Ridgeline's coaching is general training guidance, not medical advice, diagnosis or treatment.

## What the policy controls

The policy is enforced locally by plan validation (`rl_domain::plan::validate_plan`) for **every** plan: offline plans, AI proposals, adaptations and the rider's own edits. An AI model can only explain and customise within these limits; an AI proposal that breaks them is sent back once with the validation errors, and if it still fails the offline plan is used with the reason shown.

## Rider levels

Assigned from onboarding answers (`level_for`):

| Level | Rule |
|---|---|
| New to structured training (beginner) | Experience "new", or too little recent consistent riding for "developing" |
| Building consistency (developing) | ≥ 4 of the last 12 weeks consistent and ≥ 90 min/week lately |
| Returning after a break | ≥ 8 weeks off, or goal "returning" |
| Experienced | Experience "experienced", ≥ 6 consistent weeks and ≥ 150 min/week |

## Limits by level

| | Beginner | Developing | Returning | Experienced |
|---|---|---|---|---|
| Hard sessions per build week | 1 | 1 | 1 | 2 |
| Opening weeks with no hard sessions | 2 | 1 | 2 | 0 |
| Moderate sessions per week | 1 | 2 | 1 | 2 |
| Max workout difficulty (1–5) | 3 | 4 | 3 | 5 |
| Maximal assessments (ramp / 20-min test) | no | yes, after week 1 | no | yes |
| Anaerobic / sprint sessions | no | no | no | yes |
| Min rest days per week | 2 | 2 | 2 | 1 |
| Max consecutive riding days | 2 | 3 | 3 | 5 |
| Week-over-week volume growth cap | 10 % | 10 % | 10 % | 10 % |
| First-week volume cap | recent × 1.0 + 60 min | recent × 1.1 + 30 min | recent × 1.0 + 60 min | recent × 1.1 |

All levels: at least 2 calendar days between hard sessions (≥ 48 h); every 4th week is a recovery week with no hard sessions and about 65 % of the previous week's volume; plans are 1–12 weeks (default 4); sessions are only placed on days the rider marked available and never exceed that day's time; missed sessions are **skipped, not stacked** onto later days.

## Rationale and sources

**Mostly low-intensity riding, few hard sessions, spaced apart.** Endurance training in well-trained athletes is dominated by low-intensity volume with a small number of high-intensity sessions per week; hard sessions are separated by easier days. [Seiler S. What is best practice for training intensity and duration distribution in endurance athletes? *Int J Sports Physiol Perform.* 2010;5(3):276–291.] Ridgeline applies this conservatively: one hard session for non-experienced riders, two for experienced, never on consecutive days.

**Gradual progression.** Large, sudden increases in training load relative to what the athlete is used to are associated with higher injury and illness risk; increases should be gradual. [Gabbett TJ. The training–injury prevention paradox: should athletes be training smarter *and* harder? *Br J Sports Med.* 2016;50(5):273–280.] The 10 % weekly cap is a common conservative heuristic rather than a precise threshold from the literature; first-week volume is anchored to the rider's reported recent volume.

**Recovery weeks.** Periodised training alternates loading with planned unloading to allow adaptation and reduce accumulated fatigue. [Issurin VB. New horizons for the methodology and physiology of training periodization. *Sports Med.* 2010;40(3):189–206.] A 3:1 load:recovery pattern is a widely used default.

**Avoiding overreaching; respond to fatigue signals.** Persistent fatigue, poor sleep and mood disturbance are early signs of excessive load; the response is reduced load and rest. [Meeusen R, Duclos M, Foster C, et al. Prevention, diagnosis and treatment of the overtraining syndrome: joint consensus statement of the ECSS and ACSM. *Med Sci Sports Exerc.* 2013;45(1):186–205.] Simple self-reported measures (wellbeing, fatigue, soreness, sleep, stress) track the training response at least as well as many objective measures. [Saw AE, Main LC, Gastin PB. Monitoring the athlete training response: subjective self-reported measures trump commonly used objective measures: a systematic review. *Br J Sports Med.* 2016;50(5):281–291.] This underpins the readiness check and feedback-driven adaptation below.

**General activity guidance for new and returning riders.** For adults, 150–300 minutes per week of moderate activity is the public-health baseline. [Bull FC, Al-Ansari SS, Biddle S, et al. World Health Organization 2020 guidelines on physical activity and sedentary behaviour. *Br J Sports Med.* 2020;54(24):1451–1462.] Beginner and returning plans start at or slightly above the rider's current volume and build towards this range rather than starting with intensity.

**Warning symptoms stop training advice.** Chest pain or pressure, fainting, unusual breathlessness, palpitations or dizziness during or after exercise are warning signs that call for stopping and medical evaluation. [Riebe D, Franklin BA, Thompson PD, et al. Updating ACSM's recommendations for exercise preparticipation health screening. *Med Sci Sports Exerc.* 2015;47(11):2473–2479.] Ridgeline checks the readiness form, ride feedback notes and coach messages for these phrases **before** any AI request, and answers with a fixed safety message instead of training advice.

**FTP and zones.** Power targets use the 7-zone %FTP model (Z1 < 55 %, Z2 55–75 %, Z3 75–90 %, Z4 90–105 %, Z5 105–120 %, Z6 120–150 %, Z7 > 150 %) and the 20-minute test estimate FTP ≈ 95 % of 20-minute average power. [Allen H, Coggan AR, McGregor S. *Training and Racing with a Power Meter.* 3rd ed. VeloPress; 2019.] The ramp-test estimate (75 % of the best 1-minute power) is a widely used industry convention and is shown as provisional. Ridgeline never estimates FTP from age, and riders without an FTP get perceived-effort (RPE) guidance.

## Readiness check

| Input | Advice |
|---|---|
| Any warning symptom | Stop and seek care (fixed safety message); no training proposal |
| Feeling ill | Rest today |
| Pain or injury | Swap to an easier ride; ride only if pain-free |
| 3+ low signals (fatigue ≥ 4, sleep ≤ 2, stress ≥ 4, soreness ≥ 4 on 1–5 scales) | Rest |
| 1+ low signal and a hard session planned | Swap to an easier workout |
| 2 low signals | Shorten (keep ~70 %) |
| Otherwise | Proceed |

Any suggested change is a proposal the rider can accept or ignore.

## Adaptation after rides

From ride feedback (session RPE 1–10, fatigue 1–5, enjoyment 1–5, notes): exhaustion after a ride makes the next hard session easier; repeated very-hard ratings swap upcoming sessions to easier variants; consistently comfortable ratings may propose one step harder within the level limits; missed sessions are marked skipped (never rescheduled on top of other sessions). Each adaptation is a versioned proposal with an explanation; nothing changes until accepted, and an accepted change can be undone.

## What is sent to an AI model

Only with consent, and only a compact summary: goal, event date, level, experience and recent volume, weekly availability and long-ride day, FTP (if set) and power source, category preferences, upcoming planned sessions (date, workout, minutes, intensity), the policy limits for the rider's level, optional limitations (separate consent, marked as untrusted text) and optional summaries of up to 14 recent rides (date, minutes, mode, average power, RPE, fatigue — separate consent). No names, body mass, heart-rate data, locations, routes, device identifiers or second-by-second data. The exact payload is viewable in Settings → AI coach. Model output is treated as untrusted: parsed as JSON, validated, and any text is displayed as plain text.

## Changing the policy

Change `policy.rs`, bump `POLICY_VERSION`, update this document and the tests in `policy.rs` / `plan.rs`, and note the reason in the commit. Plans accepted under an older version keep working; new proposals are validated against the new version.
