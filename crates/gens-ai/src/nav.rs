//! Grid navigation: cost fields, A* and steering.
//!
//! A [`Grid`] is a rectangular cell field with per-cell walkability and
//! traversal cost. [`Grid::find_path`] runs A* over it with:
//!
//! - 4- or 8-way connectivity ([`Connectivity`]); 8-way diagonals are
//!   refused when they would **cut a corner** (both orthogonal
//!   neighbors must be walkable);
//! - deterministic tie-breaking: equal-priority candidates pop by
//!   (f, h, cell index), and neighbor expansion follows a fixed
//!   direction order — the same grid always yields the same path;
//! - entering-cell cost model: `g += unit_distance * cost(next)`.
//!
//! [`Steering`] turns paths into velocities: seek / flee / arrive and a
//! stateless path follower. All pure f32 math via the gens-math facade
//! (R-013: deterministic in-process; cross-OS bit-identity not claimed).

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use gens_math::{Vec3, vec3};
use thiserror::Error;

/// Neighbor connectivity policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Connectivity {
    /// Orthogonal neighbors only (N/E/S/W). Default.
    #[default]
    Four,
    /// Orthogonal + diagonal, with corner-cutting prevention.
    Eight,
}

/// Errors from path requests and grid edits.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum PathError {
    /// Start or goal cell lies outside the grid.
    #[error("cell ({x}, {y}) is outside the grid bounds")]
    OutOfBounds {
        /// Offending cell x.
        x: u32,
        /// Offending cell y.
        y: u32,
    },
    /// The start cell is not walkable.
    #[error("start cell ({x}, {y}) is blocked")]
    StartBlocked {
        /// Start cell x.
        x: u32,
        /// Start cell y.
        y: u32,
    },
    /// The goal cell is not walkable.
    #[error("goal cell ({x}, {y}) is blocked")]
    GoalBlocked {
        /// Goal cell x.
        x: u32,
        /// Goal cell y.
        y: u32,
    },
    /// No walkable route exists between start and goal.
    #[error("no path exists between ({sx}, {sy}) and ({gx}, {gy})")]
    NoPath {
        /// Start cell x.
        sx: u32,
        /// Start cell y.
        sy: u32,
        /// Goal cell x.
        gx: u32,
        /// Goal cell y.
        gy: u32,
    },
    /// A cell cost must be finite and greater than zero.
    #[error("cell cost must be finite and > 0, got {cost}")]
    InvalidCost {
        /// The offending cost.
        cost: f32,
    },
}

/// A walkable/cost cell field on the XZ plane.
#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    width: u32,
    height: u32,
    walkable: Vec<bool>,
    cost: Vec<f32>,
}

impl Grid {
    /// A fully walkable grid with every cost at 1.0.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        let n = (width as usize) * (height as usize);
        Self {
            width,
            height,
            walkable: vec![true; n],
            cost: vec![1.0; n],
        }
    }

    /// Grid width in cells.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Grid height in cells.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    fn idx(&self, x: u32, y: u32) -> Option<usize> {
        if x < self.width && y < self.height {
            Some((y as usize) * (self.width as usize) + (x as usize))
        } else {
            None
        }
    }

    /// `true` when the cell exists and is walkable.
    #[must_use]
    pub fn walkable(&self, x: u32, y: u32) -> bool {
        self.idx(x, y).is_some_and(|i| self.walkable[i])
    }

    /// Marks a cell walkable or blocked.
    pub fn set_walkable(&mut self, x: u32, y: u32, walkable: bool) {
        if let Some(i) = self.idx(x, y) {
            self.walkable[i] = walkable;
        }
    }

    /// Sets a cell's traversal cost (finite, > 0).
    ///
    /// # Errors
    /// [`PathError::InvalidCost`] for zero, negative or NaN costs.
    pub fn set_cost(&mut self, x: u32, y: u32, cost: f32) -> Result<(), PathError> {
        if !cost.is_finite() || cost <= 0.0 {
            return Err(PathError::InvalidCost { cost });
        }
        if let Some(i) = self.idx(x, y) {
            self.cost[i] = cost;
        }
        Ok(())
    }

    /// A cell's traversal cost (1.0 out of bounds — never used by A*).
    #[must_use]
    pub fn cost(&self, x: u32, y: u32) -> f32 {
        self.idx(x, y).map_or(1.0, |i| self.cost[i])
    }

    /// World-space center of a cell (grid lies on the XZ plane, y = 0).
    #[must_use]
    pub fn cell_center(&self, x: u32, y: u32) -> Vec3 {
        vec3(x as f32 + 0.5, 0.0, y as f32 + 0.5)
    }

    /// The cell containing a world-space position.
    #[must_use]
    pub fn world_to_cell(&self, position: Vec3) -> (i32, i32) {
        (position.x.floor() as i32, position.z.floor() as i32)
    }

    /// A* over the grid, returning the cell path start..goal inclusive.
    ///
    /// # Errors
    /// [`PathError`] for out-of-bounds endpoints, blocked endpoints, or
    /// when no route exists.
    pub fn find_path(
        &self,
        start: (u32, u32),
        goal: (u32, u32),
        connectivity: Connectivity,
    ) -> Result<Vec<(u32, u32)>, PathError> {
        let (sx, sy) = start;
        let (gx, gy) = goal;
        if self.idx(sx, sy).is_none() {
            return Err(PathError::OutOfBounds { x: sx, y: sy });
        }
        if self.idx(gx, gy).is_none() {
            return Err(PathError::OutOfBounds { x: gx, y: gy });
        }
        if !self.walkable(sx, sy) {
            return Err(PathError::StartBlocked { x: sx, y: sy });
        }
        if !self.walkable(gx, gy) {
            return Err(PathError::GoalBlocked { x: gx, y: gy });
        }

        let w = self.width as usize;
        let n = w * (self.height as usize);
        let start_i = (sy as usize) * w + (sx as usize);
        let goal_i = (gy as usize) * w + (gx as usize);

        let mut g = vec![f32::INFINITY; n];
        let mut came_from: Vec<Option<usize>> = vec![None; n];
        let mut closed = vec![false; n];
        // (Reverse((f, h, index))) - min-heap ordered by f, then h, then
        // cell index: a total, deterministic order over equal priorities.
        let mut open: BinaryHeap<Reverse<(u32, u32, usize)>> = BinaryHeap::new();
        // f and h quantized to u32 cost-units (x1024) so heap ordering is
        // exact integer comparison - no float-comparator nondeterminism.
        let h0 = self.heuristic(sx, sy, gx, gy, connectivity);
        g[start_i] = 0.0;
        open.push(Reverse((quantize(h0), quantize(h0), start_i)));

        while let Some(Reverse((_, _, current))) = open.pop() {
            if closed[current] {
                continue;
            }
            closed[current] = true;
            if current == goal_i {
                return Ok(self.reconstruct(&came_from, start_i, goal_i));
            }
            let cx = (current % w) as u32;
            let cy = (current / w) as u32;
            for (nx, ny, unit) in self.neighbors(cx, cy, connectivity) {
                let Some(ni) = self.idx(nx, ny) else {
                    continue;
                };
                if closed[ni] || !self.walkable[ni] {
                    continue;
                }
                let tentative = g[current] + unit * self.cost[ni];
                if tentative < g[ni] {
                    g[ni] = tentative;
                    came_from[ni] = Some(current);
                    let h = self.heuristic(nx, ny, gx, gy, connectivity);
                    open.push(Reverse((quantize(tentative + h), quantize(h), ni)));
                }
            }
        }
        Err(PathError::NoPath { sx, sy, gx, gy })
    }

    /// Neighbor expansion in a FIXED order: N, E, S, W, then (8-way)
    /// NE, SE, SW, NW. Diagonals are only produced when neither orthogonal
    /// neighbor of the diagonal step is blocked (no corner cutting).
    fn neighbors(&self, x: u32, y: u32, connectivity: Connectivity) -> Vec<(u32, u32, f32)> {
        let mut out = Vec::with_capacity(8);
        let west = x.checked_sub(1);
        let east = x.checked_add(1).filter(|&v| v < self.width);
        let north = y.checked_sub(1);
        let south = y.checked_add(1).filter(|&v| v < self.height);
        // Walkability probes for corner-cut prevention.
        let w_ok = west.is_some_and(|wx| self.walkable(wx, y));
        let e_ok = east.is_some_and(|ex| self.walkable(ex, y));
        let n_ok = north.is_some_and(|ny| self.walkable(x, ny));
        let s_ok = south.is_some_and(|sy| self.walkable(x, sy));

        if let Some(ny) = north {
            out.push((x, ny, 1.0));
        }
        if let Some(ex) = east {
            out.push((ex, y, 1.0));
        }
        if let Some(sy) = south {
            out.push((x, sy, 1.0));
        }
        if let Some(wx) = west {
            out.push((wx, y, 1.0));
        }
        if connectivity == Connectivity::Eight {
            const DIAG: f32 = std::f32::consts::SQRT_2;
            if let (Some(ex), Some(ny)) = (east, north)
                && e_ok
                && n_ok
            {
                out.push((ex, ny, DIAG));
            }
            if let (Some(ex), Some(sy)) = (east, south)
                && e_ok
                && s_ok
            {
                out.push((ex, sy, DIAG));
            }
            if let (Some(wx), Some(sy)) = (west, south)
                && w_ok
                && s_ok
            {
                out.push((wx, sy, DIAG));
            }
            if let (Some(wx), Some(ny)) = (west, north)
                && w_ok
                && n_ok
            {
                out.push((wx, ny, DIAG));
            }
        }
        out
    }

    /// Admissible heuristic: Manhattan for 4-way, octile for 8-way.
    fn heuristic(&self, x: u32, y: u32, gx: u32, gy: u32, connectivity: Connectivity) -> f32 {
        let dx = (gx as f32 - x as f32).abs();
        let dy = (gy as f32 - y as f32).abs();
        match connectivity {
            Connectivity::Four => dx + dy,
            Connectivity::Eight => {
                let dmin = dx.min(dy);
                let dmax = dx.max(dy);
                // octile: (max - min) + min * sqrt(2)
                (dmax - dmin) + dmin * std::f32::consts::SQRT_2
            }
        }
    }

    fn reconstruct(
        &self,
        came_from: &[Option<usize>],
        start_i: usize,
        goal_i: usize,
    ) -> Vec<(u32, u32)> {
        let w = self.width as usize;
        let mut cells = Vec::new();
        let mut current = goal_i;
        loop {
            cells.push(((current % w) as u32, (current / w) as u32));
            if current == start_i {
                break;
            }
            current = came_from[current].expect("reached cell must have a parent");
        }
        cells.reverse();
        cells
    }
}

/// Quantizes a cost to u32 units (x1024) for exact heap ordering.
fn quantize(cost: f32) -> u32 {
    // Costs here are bounded by grid size * max cell cost; saturate far
    // beyond any real grid instead of panicking on absurd inputs.
    (cost * 1024.0).round().clamp(0.0, u32::MAX as f32) as u32
}

/// Stateless steering helpers producing desired velocity vectors.
pub struct Steering;

impl Steering {
    /// Full-speed velocity toward `target`.
    #[must_use]
    pub fn seek(position: Vec3, target: Vec3, max_speed: f32) -> Vec3 {
        let to_target = target - position;
        if to_target.length_squared() < f32::EPSILON {
            return Vec3::ZERO;
        }
        to_target.normalize() * max_speed
    }

    /// Velocity away from `threat`, clamped to `max_speed`.
    #[must_use]
    pub fn flee(position: Vec3, threat: Vec3, max_speed: f32) -> Vec3 {
        -Self::seek(position, threat, max_speed)
    }

    /// Seek that decelerates inside `slow_radius`, stopping at the target.
    #[must_use]
    pub fn arrive(position: Vec3, target: Vec3, max_speed: f32, slow_radius: f32) -> Vec3 {
        let to_target = target - position;
        let distance = to_target.length();
        if distance <= f32::EPSILON {
            return Vec3::ZERO;
        }
        let speed = if distance < slow_radius {
            max_speed * (distance / slow_radius)
        } else {
            max_speed
        };
        to_target / distance * speed.min(max_speed)
    }

    /// Follows a world-space waypoint list **statelessly**.
    ///
    /// Targets the first waypoint farther than `reached_radius` from
    /// `position` and steers toward it; if every waypoint is within the
    /// radius the last one is targeted with [`Steering::arrive`].
    /// Documented limitation: an agent that has *overshot* an early
    /// waypoint will turn back toward it — stateful traversal belongs in
    /// [`PathFollower`], which never re-targets a passed waypoint.
    #[must_use]
    pub fn follow_path(
        waypoints: &[Vec3],
        position: Vec3,
        max_speed: f32,
        reached_radius: f32,
    ) -> Vec3 {
        let Some(_) = waypoints.first() else {
            return Vec3::ZERO;
        };
        let mut target_idx = waypoints.len() - 1;
        for (i, waypoint) in waypoints.iter().enumerate() {
            if (waypoint - position).length() > reached_radius {
                target_idx = i;
                break;
            }
        }
        let target = &waypoints[target_idx];
        if target_idx == waypoints.len() - 1 {
            Self::arrive(position, *target, max_speed, reached_radius.max(0.25))
        } else {
            Self::seek(position, *target, max_speed)
        }
    }
}

/// Stateful waypoint consumer: never re-targets a waypoint it has passed.
///
/// [`PathFollower`] advances an internal cursor while the current
/// waypoint sits within `reached_radius`, steers toward the cursor
/// waypoint (seek), and switches to [`Steering::arrive`] for the final
/// waypoint. Deterministic: the cursor only ever moves forward.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathFollower {
    cursor: usize,
}

impl Default for PathFollower {
    fn default() -> Self {
        Self::new()
    }
}

impl PathFollower {
    /// A follower parked at the first waypoint.
    #[must_use]
    pub fn new() -> Self {
        Self { cursor: 0 }
    }

    /// Restarts traversal from the first waypoint.
    pub fn reset(&mut self) {
        self.cursor = 0;
    }

    /// Current waypoint index (diagnostics and tests).
    #[must_use]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// `true` when the final waypoint was reached and arrival speed is 0.
    #[must_use]
    pub fn arrived(&self, waypoints: &[Vec3], position: Vec3, reached_radius: f32) -> bool {
        match waypoints.last() {
            Some(last) => {
                self.cursor >= waypoints.len() - 1 && (*last - position).length() <= reached_radius
            }
            None => true,
        }
    }

    /// Advances the cursor over `waypoints` and returns the desired velocity.
    #[must_use]
    pub fn steer(
        &mut self,
        waypoints: &[Vec3],
        position: Vec3,
        max_speed: f32,
        reached_radius: f32,
    ) -> Vec3 {
        let Some(_) = waypoints.first() else {
            return Vec3::ZERO;
        };
        while self.cursor + 1 < waypoints.len()
            && (waypoints[self.cursor] - position).length() <= reached_radius
        {
            self.cursor += 1;
        }
        let target = &waypoints[self.cursor];
        if self.cursor == waypoints.len() - 1 {
            Steering::arrive(position, *target, max_speed, reached_radius.max(0.25))
        } else {
            Steering::seek(position, *target, max_speed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_defaults_are_walkable_cost_one() {
        let g = Grid::new(4, 3);
        assert_eq!((g.width(), g.height()), (4, 3));
        assert!(g.walkable(3, 2));
        assert_eq!(g.cost(0, 0), 1.0);
        assert!(!g.walkable(4, 2), "out of bounds reads blocked");
    }

    #[test]
    fn set_cost_rejects_non_positive_and_nan() {
        let mut g = Grid::new(2, 2);
        assert_eq!(
            g.set_cost(0, 0, 0.0),
            Err(PathError::InvalidCost { cost: 0.0 })
        );
        assert!(matches!(
            g.set_cost(0, 0, f32::NAN),
            Err(PathError::InvalidCost { cost }) if cost.is_nan()
        ));
        assert!(g.set_cost(1, 1, 2.5).is_ok());
        assert_eq!(g.cost(1, 1), 2.5);
    }

    #[test]
    fn straight_line_four_way_path_is_manhattan_exact() {
        let g = Grid::new(5, 5);
        let path = g
            .find_path((1, 1), (3, 1), Connectivity::Four)
            .expect("open grid");
        assert_eq!(path.len(), 3); // 2 steps + inclusive goal
        assert_eq!(path[0], (1, 1));
        assert_eq!(path[1], (2, 1));
        assert_eq!(path[2], (3, 1));
    }

    #[test]
    fn path_routes_around_a_wall() {
        let mut g = Grid::new(5, 3);
        // Wall down the middle except a gap at the top.
        for y in 0..3 {
            g.set_walkable(2, y, false);
        }
        g.set_walkable(2, 0, true);
        let path = g
            .find_path((0, 2), (4, 2), Connectivity::Four)
            .expect("gap exists");
        assert_eq!(path[0], (0, 2));
        assert_eq!(*path.last().expect("non-empty"), (4, 2));
        for &(x, y) in &path {
            assert!(g.walkable(x, y), "path may only cross walkable cells");
        }
        // Minimal detour through (2,0): path length must be 9 cells.
        assert_eq!(path.len(), 9);
    }

    #[test]
    fn blocked_endpoints_are_rejected() {
        let mut g = Grid::new(3, 3);
        g.set_walkable(0, 0, false);
        g.set_walkable(2, 2, false);
        assert_eq!(
            g.find_path((0, 0), (2, 2), Connectivity::Four),
            Err(PathError::StartBlocked { x: 0, y: 0 })
        );
        assert_eq!(
            g.find_path((1, 1), (2, 2), Connectivity::Four),
            Err(PathError::GoalBlocked { x: 2, y: 2 })
        );
        assert!(matches!(
            g.find_path((9, 9), (1, 1), Connectivity::Four),
            Err(PathError::OutOfBounds { x: 9, y: 9 })
        ));
    }

    #[test]
    fn no_path_when_fully_walled_off() {
        let mut g = Grid::new(5, 5);
        for y in 0..5 {
            g.set_walkable(2, y, false);
        }
        assert_eq!(
            g.find_path((0, 2), (4, 2), Connectivity::Four),
            Err(PathError::NoPath {
                sx: 0,
                sy: 2,
                gx: 4,
                gy: 2
            })
        );
        // Same with diagonals: the wall still seals the grid.
        assert_eq!(
            g.find_path((0, 2), (4, 2), Connectivity::Eight),
            Err(PathError::NoPath {
                sx: 0,
                sy: 2,
                gx: 4,
                gy: 2
            })
        );
    }

    #[test]
    fn start_equals_goal_returns_single_cell() {
        let g = Grid::new(4, 4);
        let path = g
            .find_path((2, 2), (2, 2), Connectivity::Four)
            .expect("trivial");
        assert_eq!(path, vec![(2, 2)]);
    }

    #[test]
    fn eight_way_refuses_corner_cuts() {
        let mut g = Grid::new(3, 3);
        // Block the two orthogonal neighbors of the (0,1)->(1,2) diagonal.
        g.set_walkable(1, 1, false);
        g.set_walkable(0, 2, false);
        let path = g
            .find_path((0, 1), (1, 2), Connectivity::Eight)
            .expect("still connected the long way");
        let has_diagonal = path.windows(2).any(|pair| {
            let dx = (pair[0].0 as i32 - pair[1].0 as i32).abs();
            let dy = (pair[0].1 as i32 - pair[1].1 as i32).abs();
            dx == 1 && dy == 1
        });
        assert!(!has_diagonal, "no diagonal step may cut a corner: {path:?}");
    }

    #[test]
    fn eight_way_takes_the_diagonal_when_legal() {
        let g = Grid::new(4, 4);
        let path = g
            .find_path((0, 0), (2, 2), Connectivity::Eight)
            .expect("open grid");
        assert_eq!(path.len(), 3, "two diagonal steps reach the goal");
        assert_eq!(path[1], (1, 1));
    }

    #[test]
    fn same_grid_same_path_deterministic_tiebreak() {
        let mut g = Grid::new(6, 4);
        // Expensive middle column makes two equal-cost detours (up/down).
        for y in 0..4 {
            g.set_cost(2, y, 5.0).expect("valid");
        }
        let a = g
            .find_path((0, 1), (4, 2), Connectivity::Four)
            .expect("path");
        let b = g
            .find_path((0, 1), (4, 2), Connectivity::Four)
            .expect("path");
        assert_eq!(a, b, "identical requests must produce identical paths");
    }

    #[test]
    fn weighted_cost_shifts_the_route() {
        let mut g = Grid::new(3, 1);
        g.set_cost(1, 0, 100.0).expect("valid");
        // Direct step through the expensive cell is still the only route,
        // so the path is unchanged - but on a 2-row grid the detour wins.
        let mut g2 = Grid::new(3, 2);
        g2.set_cost(1, 0, 100.0).expect("valid");
        let path = g2
            .find_path((0, 0), (2, 0), Connectivity::Four)
            .expect("connected");
        assert!(
            path.contains(&(1, 1)),
            "cheap row must be preferred: {path:?}"
        );
        let _ = g;
    }

    #[test]
    fn cell_centers_and_world_mapping_round_trip() {
        let g = Grid::new(10, 10);
        assert_eq!(g.cell_center(0, 0), vec3(0.5, 0.0, 0.5));
        assert_eq!(g.cell_center(3, 7), vec3(3.5, 0.0, 7.5));
        let world = g.cell_center(6, 2);
        assert_eq!(g.world_to_cell(world), (6, 2));
    }

    #[test]
    fn seek_is_full_speed_toward_target() {
        let v = Steering::seek(vec3(0.0, 0.0, 0.0), vec3(10.0, 0.0, 0.0), 4.0);
        assert_eq!(v, vec3(4.0, 0.0, 0.0));
        // Zero distance yields zero velocity, not NaN.
        assert_eq!(Steering::seek(Vec3::ONE, Vec3::ONE, 4.0), Vec3::ZERO);
    }

    #[test]
    fn flee_is_negated_seek() {
        let v = Steering::flee(vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 5.0), 2.0);
        assert_eq!(v, vec3(0.0, 0.0, -2.0));
    }

    #[test]
    fn arrive_decelerates_inside_slow_radius() {
        let full = Steering::arrive(vec3(0.0, 0.0, 0.0), vec3(20.0, 0.0, 0.0), 5.0, 2.0);
        assert_eq!(full, vec3(5.0, 0.0, 0.0));
        let inside = Steering::arrive(vec3(18.5, 0.0, 0.0), vec3(20.0, 0.0, 0.0), 5.0, 2.0);
        assert!(
            inside.length() < 5.0 && inside.x > 0.0,
            "inside the radius the speed must scale down: {inside:?}"
        );
        let there = Steering::arrive(vec3(20.0, 0.0, 0.0), vec3(20.0, 0.0, 0.0), 5.0, 2.0);
        assert_eq!(there, Vec3::ZERO);
    }

    #[test]
    fn follow_path_stateless_semantics_are_documented() {
        let waypoints = vec![
            vec3(1.0, 0.0, 0.0),
            vec3(5.0, 0.0, 0.0),
            vec3(9.0, 0.0, 0.0),
        ];
        // Standing on waypoint 0: target becomes waypoint 1.
        let v = Steering::follow_path(&waypoints, vec3(1.0, 0.0, 0.0), 3.0, 0.5);
        assert_eq!(v, vec3(3.0, 0.0, 0.0));
        // Documented stateless limitation: an agent that has overshot the
        // early waypoints turns back toward the first one beyond radius.
        let v = Steering::follow_path(&waypoints, vec3(8.75, 0.0, 0.0), 3.0, 0.5);
        assert_eq!(v, vec3(-3.0, 0.0, 0.0));
        // All waypoints within radius (clustered near the agent): arrive
        // at the last one with scaled speed.
        let clustered = vec![
            vec3(8.7, 0.0, 0.0),
            vec3(8.9, 0.0, 0.0),
            vec3(9.1, 0.0, 0.0),
        ];
        let v = Steering::follow_path(&clustered, vec3(8.9, 0.0, 0.0), 3.0, 0.5);
        assert!(v.x >= 0.0 && v.x < 3.0, "arrival speed scales down: {v:?}");
        // Empty path is a safe no-op.
        assert_eq!(Steering::follow_path(&[], Vec3::ZERO, 3.0, 0.5), Vec3::ZERO);
    }

    #[test]
    fn path_follower_traverses_forward_and_arrives() {
        let waypoints = vec![
            vec3(1.0, 0.0, 0.0),
            vec3(5.0, 0.0, 0.0),
            vec3(9.0, 0.0, 0.0),
        ];
        let mut f = PathFollower::new();
        // At waypoint 0: cursor advances to 1, full speed toward (5,0,0).
        let v = f.steer(&waypoints, vec3(1.0, 0.0, 0.0), 3.0, 0.5);
        assert_eq!(f.cursor(), 1);
        assert_eq!(v, vec3(3.0, 0.0, 0.0));
        // Inside waypoint 1's radius: cursor advances to the last waypoint
        // and steering switches to arrival semantics toward it.
        let v = f.steer(&waypoints, vec3(5.4, 0.0, 0.0), 3.0, 0.5);
        assert_eq!(f.cursor(), 2);
        assert_eq!(v, vec3(3.0, 0.0, 0.0));
        // Enter the final waypoint's slow radius: arrival deceleration.
        let v = f.steer(&waypoints, vec3(8.75, 0.0, 0.0), 3.0, 0.5);
        assert_eq!(f.cursor(), 2);
        assert!(v.x > 0.0 && v.x < 3.0, "final approach decelerates: {v:?}");
        // Touch the goal: arrived() reports true.
        let _ = f.steer(&waypoints, vec3(9.0, 0.0, 0.0), 3.0, 0.5);
        assert!(f.arrived(&waypoints, vec3(9.0, 0.0, 0.0), 0.5));
        // Reset returns to the start.
        f.reset();
        assert_eq!(f.cursor(), 0);
    }

    #[test]
    fn path_to_velocity_pipeline_is_deterministic() {
        let g = Grid::new(8, 8);
        let cells = g
            .find_path((0, 0), (5, 3), Connectivity::Eight)
            .expect("path");
        let world: Vec<Vec3> = cells.iter().map(|&(x, y)| g.cell_center(x, y)).collect();
        let pos = g.cell_center(0, 0);
        let run = || {
            let v = Steering::follow_path(&world, pos, 2.0, 0.35);
            [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()]
        };
        assert_eq!(run(), run());
    }
}
