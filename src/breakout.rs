//! M35 Veil Breakout — paddle (arrow keys), ball, brick grid, levels, score.
//! Modern dark palette. Ball advances on the desktop timer tick.

use crate::wm::{App, Window};
use crate::{kprintln, timer};
use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

const W: i32 = 280;
const H: i32 = 320;
const TOP: i32 = 24; // score bar
const COLS: i32 = 10;
const ROWS: i32 = 6;
const BRICK_W: i32 = W / COLS;
const BRICK_H: i32 = 14;
const PADDLE_W: i32 = 48;
const PADDLE_H: i32 = 8;
const BALL: i32 = 6;
const STEP: u64 = 1; // ticks per frame (~50 fps)

const BG: u32 = 0xff0d_0d0d;
const PADDLE: u32 = 0xff5b_8af0;
const BALL_C: u32 = 0xffe8_e8e8;
const TEXT: u32 = 0xffe8_e8e8;
const MUTED: u32 = 0xff88_8888;
const ROW_COLORS: [u32; 6] =
    [0xffe0_604d, 0xffe0_9a4d, 0xffd6_c84a, 0xff5b_b06a, 0xff5b_8af0, 0xff9a_6ad6];

pub struct BreakoutState {
    bricks: Vec<bool>, // COLS*ROWS
    paddle_x: i32,
    bx: i32,
    by: i32,
    vx: i32,
    vy: i32,
    score: u32,
    lives: u32,
    level: u32,
    over: bool,
    launched: bool,
    next_tick: u64,
}

impl BreakoutState {
    pub fn new() -> BreakoutState {
        kprintln!("BREAKOUT: new game");
        let mut st = BreakoutState {
            bricks: vec![true; (COLS * ROWS) as usize],
            paddle_x: W / 2 - PADDLE_W / 2,
            bx: 0,
            by: 0,
            vx: 2,
            vy: -3,
            score: 0,
            lives: 3,
            level: 1,
            over: false,
            launched: false,
            next_tick: 0,
        };
        st.reset_ball();
        st
    }

    fn reset_ball(&mut self) {
        self.bx = W / 2;
        self.by = TOP + H - 40;
        self.vx = 2;
        self.vy = -3;
        self.launched = false;
    }

    fn advance(&mut self) {
        if self.over || !self.launched {
            return;
        }
        self.bx += self.vx;
        self.by += self.vy;
        // Walls.
        if self.bx <= 0 {
            self.bx = 0;
            self.vx = -self.vx;
        }
        if self.bx >= W - BALL {
            self.bx = W - BALL;
            self.vx = -self.vx;
        }
        if self.by <= TOP {
            self.by = TOP;
            self.vy = -self.vy;
        }
        // Paddle.
        let paddle_y = TOP + H - PADDLE_H - 4;
        if self.by + BALL >= paddle_y
            && self.by + BALL <= paddle_y + PADDLE_H + 4
            && self.bx + BALL >= self.paddle_x
            && self.bx <= self.paddle_x + PADDLE_W
            && self.vy > 0
        {
            self.vy = -self.vy;
            // English: deflect based on where it hit the paddle.
            let hit = (self.bx + BALL / 2) - (self.paddle_x + PADDLE_W / 2);
            self.vx = (hit / 8).clamp(-4, 4);
            if self.vx == 0 {
                self.vx = 1;
            }
        }
        // Bricks.
        let col = (self.bx + BALL / 2) / BRICK_W;
        let row = (self.by + BALL / 2 - TOP) / BRICK_H;
        if (0..COLS).contains(&col) && (0..ROWS).contains(&row) {
            let i = (row * COLS + col) as usize;
            if self.bricks[i] {
                self.bricks[i] = false;
                self.vy = -self.vy;
                self.score += 10;
                kprintln!("BREAKOUT: score {}", self.score);
                if self.bricks.iter().all(|b| !b) {
                    self.level += 1;
                    self.bricks = vec![true; (COLS * ROWS) as usize];
                    self.reset_ball();
                    kprintln!("BREAKOUT: level {} cleared!", self.level - 1);
                }
            }
        }
        // Fell past the paddle: lose a life.
        if self.by > TOP + H {
            self.lives = self.lives.saturating_sub(1);
            if self.lives == 0 {
                self.over = true;
                kprintln!("BREAKOUT: game over, score {}", self.score);
            } else {
                self.reset_ball();
            }
        }
    }
}

pub fn key(win: &mut Window, code: u16) -> bool {
    const LEFT: u16 = 105;
    const RIGHT: u16 = 106;
    const A: u16 = 30;
    const D: u16 = 32;
    const SPACE: u16 = 57;
    const R: u16 = 19;
    let App::Breakout(st) = &mut win.app else { return false };
    match code {
        LEFT | A => st.paddle_x = (st.paddle_x - 16).max(0),
        RIGHT | D => st.paddle_x = (st.paddle_x + 16).min(W - PADDLE_W),
        SPACE => {
            if st.over {
                *st = BreakoutState::new();
            } else {
                st.launched = true;
            }
        }
        R => *st = BreakoutState::new(),
        _ => return false,
    }
    render(win);
    true
}

pub fn tick(win: &mut Window, now: u64) -> bool {
    {
        let App::Breakout(st) = &mut win.app else { return false };
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
    let (bricks, px, bx, by, score, lives, level, over, launched) = {
        let App::Breakout(st) = &win.app else { return };
        (st.bricks.clone(), st.paddle_x, st.bx, st.by, st.score, st.lives, st.level, st.over, st.launched)
    };
    let fb = win.canvas_fb();
    fb.clear(BG);
    fb.draw_string(6, 4, &format!("SCORE {score}"), TEXT, None);
    fb.draw_string(150, 4, &format!("LV {level}  LIVES {lives}"), MUTED, None);
    let put = |x: i32, y: i32, w: i32, h: i32, c: u32| {
        if x >= 0 && y >= 0 {
            fb.fill_rect(x as usize, y as usize, w.max(0) as usize, h.max(0) as usize, c);
        }
    };
    for r in 0..ROWS {
        for c in 0..COLS {
            if bricks[(r * COLS + c) as usize] {
                put(c * BRICK_W + 1, TOP + r * BRICK_H + 1, BRICK_W - 2, BRICK_H - 2, ROW_COLORS[r as usize]);
            }
        }
    }
    put(px, TOP + H - PADDLE_H - 4, PADDLE_W, PADDLE_H, PADDLE);
    put(bx, by, BALL, BALL, BALL_C);
    if over {
        fb.draw_string(70, (TOP + H / 2) as usize, "GAME OVER", BALL_C, Some(BG));
        fb.draw_string(48, (TOP + H / 2 + 18) as usize, "SPACE to restart", MUTED, Some(BG));
    } else if !launched {
        fb.draw_string(50, (TOP + H / 2) as usize, "SPACE to launch", MUTED, Some(BG));
    }
}
