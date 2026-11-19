# Predicted traffic: a "timetable" for driving

!!! note "Status: plan, not built"

## The goal

A prediction (for example SonoFlow's hourly flow for each road edge, with its weekly shape) gives the travel time of
each road edge **for each time**. A driving route that starts at 08:00 must use the times of 08:00 on its first road
edges, and the times of 08:20 on the road edges that it reaches 20 minutes later, as a timetable does for a bus.

## Three ways

| Way | Arc cost | Preparation | Query | Exact? |
|---|---|---|---|---|
| **1. Time slices** | one cost for each slice (for example each hour of a weekday, Saturday, Sunday: 3 × 24 = 72 slices) | one cost rebuild for each slice, on the same structure | the normal query with the costs of the departure slice | the whole route uses the costs of its departure time |
| **2. Slices along the route** | as 1 | as 1 | a search that changes to the next slice when its time passes the slice border, on the base graph near the route | yes, for the slices; slower than 1 |
| **3. Time-dependent costs** | a function of the entry time for each arc (a piecewise-linear profile) | shortcuts with profiles (as in time-dependent contraction hierarchies) | a search with profiles | yes; heavy preparation and memory |

## The plan: way 1 first, then way 2 where needed

**Way 1** fits what tileroute already has:

- The structure (tiles, levels, shortcut places) is made one time.
- Each slice is one rebuild of the costs ([Updates](updates.md)): seconds for a city, minutes for a country (to be
  measured).
- Each tile file holds the costs of all slices, or one file for each slice; a query reads only the slice of its
  departure time.
- For trips shorter than one slice (most city trips), the error is small.

**Way 2** for long trips: the hierarchy with the costs of each slice gives a **lower bound** (the smallest cost over all
slices). A search that knows the time on each road edge then runs only near the route that the lower bound shows. This
is exact, and it reads only the tiles near the route.

**Way 3** only if way 2 is not exact enough or too slow: it costs the most memory and preparation.

## The data

| Data | Source |
|---|---|
| Flow for each road edge and each hour of the week | SonoFlow (`areas/<area>/observations/`, the per-edge hourly model) |
| Travel time from flow | a speed-flow function for each road class (for example the BPR function) |
| Slices | from the weekly shape of the prediction: weekday, Saturday, Sunday × 24 hours |

The travel time must keep the **FIFO** rule: leaving later never makes you arrive earlier. Without it, a search can
give wrong routes. The conversion from flow to time must keep this rule.

## From hourly values to a time function

The prediction gives one travel time τ(e, h) for each road edge e and each hour h. A bus gives departures at fixed
times. Both become the same kind of arc: an **arrival function** "enter at time t, arrive at time f(t)".

| Arc | Arrival function f(t) |
|---|---|
| Road edge with predicted times | t + τ(e, t), with τ between the hourly values (see below) |
| Bus or train between two stops | the arrival time of the first departure at t or later |
| Walking | t + (a fixed time) |

**Do not use the hourly values as steps.** With steps, the FIFO rule breaks at the hour border:

| Enter at | τ | Arrive at |
|---|---|---|
| 07:59 | 10 min (the 07:00 value) | 08:09 |
| 08:00 | 5 min (the 08:00 value) | 08:05: earlier, although you entered later |

So τ goes **linearly** from the value of one hour (at the middle of that hour) to the value of the next hour. FIFO holds
when τ does not fall faster than the clock: the slope of τ must be more than −1 (for example, it cannot fall by more than
60 minutes in one hour). Real hourly travel times keep this easily; the preparation checks it for each road edge.

With FIFO arcs, a time-dependent Dijkstra (the label of a node is its earliest arrival time) is exact, and the same search
handles road edges, walking and buses: only the arrival function of the arc is different.

## With the other parts

| Part | How it joins |
|---|---|
| [Weights](weights.md) | the time item has a value for each slice; the other items (turns, lights) stay the same |
| [Modes](modes.md) | only driving (and buses on the road, if wanted) use the predicted slices |
| [Timetable](timetable.md) | a door-to-door trip by car and train uses the slice of the car part and the timetable of the train part |

## Checks

| Check | Rule |
|---|---|
| Way 1 | for each slice, equal to duckOSM's router with the costs of that slice |
| Way 2 | equal to a plain time-dependent Dijkstra on the base graph with the slices, for random pairs and departure times |
