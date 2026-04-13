# Mammoth UI Map (MVP Screens)

| Screen | Route | Purpose |
|--------|-------|---------|
| 1. Landing / Discovery | `/` | Token cards, sort tabs, casino energy |
| 2. Project Page (Open) | `/token/:mint` | Chart, cycle panel, buy modal, tabs |
| 3. Project Page (Closed) | `/token/:mint` | Trading via aggregator, fee notice |
| 4. Creator Dashboard | `/creator` | Supply status, active cycle, past cycles, treasury |
| 5. Launch Wizard | `/launch` | 3-step deploy: basics → supply mode → allocation |
| 6. Create Cycle | `/creator/cycle/new` | Allocation, curve, rights, treasury routing |
| 7. Open Cycle Confirm | modal | Lock params, take snapshot, go live |
| 8. End Cycle Early | modal | Confirm early termination |

## Key UX Decisions

- No wallet required to browse
- "Next price jump in X tokens" is the gamification heartbeat
- Rights shown only when wallet connected
- No math exposed in the UI
- Pump.fun-familiar flow intentionally

## Design System

| Token | Value |
|-------|-------|
| Primary accent | Mammoth Orange `#FF9F1C` |
| Background | `#0B0E11` |
| Font | Inter |
| Mode | Dark mode first, high contrast |
| Aesthetic | pump.fun + Birdeye energy, not DeFi governance sludge |
