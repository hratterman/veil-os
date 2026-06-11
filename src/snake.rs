//! M35 Snake — arrow-key controls, score, high score persisted to SNAKE.TXT.
//! Uses the modern dark palette. Advances on the desktop timer tick.

use crate::wm::{App, Window};
use crate::{fs, kprintln, timer};
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

const COLS: i32 = 18;
const ROWS: i32 = 18;
const CELL: usize = 14;
const TOP: usize = 24; // score bar height
const STEP: u64 = 7; // ticks between moves (~140 ms)

const BG: u32 = 0xff0d_0d0d;
const GRID: u32 = 0xff15_1515;
const SNAKE: u32 = 0xff5b_8af0;
const HEAD: u32 = 0xff8f_b4ff;
const FOOD: u32 = 0xffe0_604d;
const TEXT: u32 = 0xffe8_e8e8;
const MUTED: u32 = 0xff88_8888;

pub struct SnakeState {
    body: Vec<(i32, i32)>, // head first
    dir: (i32, i32),
    pending: (i32, i32),
    food: (i32, i32),
    score: u32,
    high: u32,
    over: bool,
    next_tick: u64,
    rng: u64,
}

impl SnakeState {
    pub fn new() -> SnakeState {
        let high = fs::read_file("SNAKE.TXT")
            .and_then(|d| core::str::from_utf8(&d).ok()?.trim().parse::<u32>().ok())
            .unwrap_or(0);
        let mut st = SnakeState {
            body: vec![(COLS / 2, ROWS / 2), (COLS / 2 - 1, ROWS / 2), (COLS / 2 - 2, ROWS / 2)],
            dir: (1, 0),
            pending: (1, 0),
            food: (0, 0),
            score: 0,
            high,
            over: false,
            next_tick: 0,
            rng: timer::ticks().wrapping_mul(2654435761).wrapping_add(12345) | 1,
        };
        // The first food sits straight ahead of the snake (deterministic first
        // bite); every later one is random.
        st.food = (COLS / 2 + 3, ROWS / 2);
        kprintln!("SNAKE: new game, high score {high}");
        st
    }

    fn rand(&mut self) -> u64 {
        // xorshift64
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }

    fn place_food(&mut self) {
        loop {
            let fx = (self.rand() % COLS as u64) as i32;
            let fy = (self.rand() % ROWS as u64) as i32;
            if !self.body.contains(&(fx, fy)) {
                self.food = (fx, fy);
                return;
            }
        }
    }

    fn restart(&mut self) {
        self.body = vec![(COLS / 2, ROWS / 2), (COLS / 2 - 1, ROWS / 2), (COLS / 2 - 2, ROWS / 2)];
        self.dir = (1, 0);
        self.pending = (1, 0);
        self.score = 0;
        self.over = false;
        self.place_food();
    }

    fn advance(&mut self) {
        if self.over {
            return;
        }
        // Apply the pending direction unless it reverses onto the neck.
        if self.pending != (-self.dir.0, -self.dir.1) {
            self.dir = self.pending;
        }
        let head = self.body[0];
        let nh = (head.0 + self.dir.0, head.1 + self.dir.1);
        // Wall or self collision ends the game.
        if nh.0 < 0 || nh.0 >= COLS || nh.1 < 0 || nh.1 >= ROWS || self.body.contains(&nh) {
            self.over = true;
            if self.score > self.high {
                self.high = self.score;
                let _ = fs::write_file("SNAKE.TXT", format!("{}", self.high).as_bytes());
                kprintln!("SNAKE: new high score {} saved to SNAKE.TXT", self.high);
            }
            kprintln!("SNAKE: game over, score {}", self.score);
            return;
        }
        self.body.insert(0, nh);
        if nh == self.food {
            self.score += 1;
            self.place_food();
            kprintln!("SNAKE: score {}", self.score);
        } else {
            self.body.pop();
        }
    }
}

/// Direction keys (arrows / WASD); R restarts after game over.
pub fn key(win: &mut Window, code: u16) -> bool {
    const LEFT: u16 = 105;
    const RIGHT: u16 = 106;
    const UP: u16 = 103;
    const DOWN: u16 = 108;
    const W: u16 = 17;
    const A: u16 = 30;
    const S: u16 = 31;
    const D: u16 = 32;
    const R: u16 = 19;
    let App::Snake(st) = &mut win.app else { return false };
    match code {
        LEFT | A => st.pending = (-1, 0),
        RIGHT | D => st.pending = (1, 0),
        UP | W => st.pending = (0, -1),
        DOWN | S => st.pending = (0, 1),
        R if st.over => st.restart(),
        _ => return false,
    }
    render(win);
    true
}

/// Advance on the timer tick (every STEP ticks).
pub fn tick(win: &mut Window, now: u64) -> bool {
    {
        let App::Snake(st) = &mut win.app else { return false };
        if st.over || now < st.next_tick {
            return false;
        }
        st.next_tick = now + STEP;
        st.advance();
    }
    render(win);
    true
}

pub fn render(win: &mut Window) {
    let (body, food, score, high, over) = {
        let App::Snake(st) = &win.app else { return };
        (st.body.clone(), st.food, st.score, st.high, st.over)
    };
    let fb = win.canvas_fb();
    fb.clear(BG);
    // Score bar.
    fb.draw_string(6, 4, &format!("SCORE {score}"), TEXT, None);
    fb.draw_string(150, 4, &format!("HIGH {high}"), MUTED, None);
    // Faint grid.
    for c in 0..=COLS {
        fb.fill_rect(c as usize * CELL, TOP, 1, ROWS as usize * CELL, GRID);
    }
    for r in 0..=ROWS {
        fb.fill_rect(0, TOP + r as usize * CELL, COLS as usize * CELL, 1, GRID);
    }
    let draw_cell = |x: i32, y: i32, color: u32| {
        if x >= 0 && y >= 0 && x < COLS && y < ROWS {
            fb.fill_rect(x as usize * CELL + 1, TOP + y as usize * CELL + 1, CELL - 2, CELL - 2, color);
        }
    };
    draw_cell(food.0, food.1, FOOD);
    for (i, &(x, y)) in body.iter().enumerate() {
        draw_cell(x, y, if i == 0 { HEAD } else { SNAKE });
    }
    if over {
        fb.draw_string(40, TOP + 110, "GAME OVER", FOOD, Some(BG));
        fb.draw_string(34, TOP + 130, "press R to play", MUTED, Some(BG));
    }
}
