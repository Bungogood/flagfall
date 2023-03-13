#![warn(clippy::all, clippy::pedantic, clippy::nursery)]
#![allow(dead_code)]

use log::{info, error};
use shakmaty::{
    san::San, Bitboard, Chess, Color, File, Move, Position, Rank, Role,
    Square,
};
use std::io::{BufReader, BufRead};
use std::io::Write;

// handle exe paths on windows & unix
#[cfg(windows)]
const OPPONENT_WRAPPER_EXE_PATH: &str = "opponent-wrapper.exe";
#[cfg(unix)]
const OPPONENT_WRAPPER_EXE_PATH: &str = "opponent-wrapper";

// 1. SETUP BOARD (kinda handwaved, user probably does it)
// 2. SETUP GAME PARAMETERS (time control, human playing colour, etc)
// 3. READ REED-SWITCH OUTPUT
// 4. UPDATE INTERNAL STATE FROM RSWITCH
// 5. [MAYBE] UPDATE LEDS
// 6. GOTO 3 UNTIL DONE
// 7. OUTPUT MOVE TO OPPONENT WRAPPER
// 8. RECEIVE MOVE FROM OPPONENT
// 9. CONVERT MOVE TO MOVEMENT STEPS
// 10. SEND STEPS TO LEVY'S PROGRAM
// 11. GOTO 3 UNTIL GAME ENDS
// 12. EXIT

fn main() {
    env_logger::init();

    // STEP 1: SETUP BOARD
    let mut pos = Chess::default();
    let (mut captured_whites, mut captured_blacks) = (0u8, 0u8);
    let mut state = State::Idle;
    info!("Entered starting position: {fen}", fen = pos.board());

    // STEP 2: SETUP GAME PARAMETERS
    let mut opponent_wrapper_proc = std::process::Command::new(OPPONENT_WRAPPER_EXE_PATH)
        .arg("-e")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("Failed to spawn opponent-wrapper process");
    let opponent_wrapper_stdout = BufReader::new(opponent_wrapper_proc.stdout.take().unwrap());
    let mut opponent_wrapper_stdin = opponent_wrapper_proc.stdin.take().unwrap();
    let mut stdout_lines = opponent_wrapper_stdout.lines();

    // the opponent wrapper gives two prompts on boot, we need to pipe them through and pipe the responses back
    let mut user_input = String::new();
    let first_line = stdout_lines.next().unwrap().unwrap();
    println!("{first_line}");
    std::io::stdin().read_line(&mut user_input).unwrap();
    write!(opponent_wrapper_stdin, "{user_input}").unwrap();
    let second_line = stdout_lines.next().unwrap().unwrap();
    println!("{second_line}");
    user_input.clear();
    std::io::stdin().read_line(&mut user_input).unwrap();
    write!(opponent_wrapper_stdin, "{user_input}").unwrap();
    let mut send_line = |line: &str| {
        let res = writeln!(opponent_wrapper_stdin, "{line}");
        if let Err(e) = res {
            error!("Failed to send line to opponent wrapper: {e}");
        }
    };
    let mut recv_line = || {
        stdout_lines.next().unwrap().unwrap()
    };

    // Right now the program is set to loop through the input from the reed switches ONLY
    loop {
        if pos.is_game_over() {
            info!("game ended with {}", pos.outcome().unwrap());
            break;
        }
        loop {
            // STEP 3: READ REED-SWITCH OUTPUT
            let mut line = String::new();
            let newstate = state;

            // This is input from REED SWITCHES
            std::io::stdin().read_line(&mut line).unwrap();
            if user_input == "\x04" {
                info!("received EOF from opponent wrapper, exiting");
                return;
            }
            let user_input = line.trim();
            info!("received line: {user_input}");
            if user_input == "-1" {
                break;
            }

            let mv;
            (state, mv) = update_state(&pos, user_input.parse::<u32>().unwrap(), newstate);
            let copied_pos = pos.clone();
            if let Some(mv) = mv {
                info!("got full move, playing {mv}");
                pos = copied_pos.play(&mv).unwrap();
                let move_san = San::from_move(&pos, &mv).to_string();
                info!("sending move {move_san} to opponent wrapper");
                send_line(&move_san);
                break;
            }
        }

        let move_from_opponent = recv_line();
        let san: San = move_from_opponent.parse().expect("Moves from opponent should always be valid SAN.");
        let mv = san.to_move(&pos).expect("SANs from opponent should always be legal moves.");
        info!("got move {mv} from opponent wrapper");

        // STEP 9: CONVERT MOVE TO MOVEMENT STEPS

        let steps = move_to_steps(mv, pos.turn(), f64::from(captured_whites), f64::from(captured_blacks));
        info!("produced steps: {steps:?}", steps = steps);
        
        // [TODO] STEP 10: SEND TO ARDUINO?
    }

    //The input of SAN is gonna access through this method:
    //convert_san_to_steps(INPUT, pos, captured_blacks, captured_whites)
    //the method also gives an output for CORE-XY in the form of a list of structs
    //TODO: make sure that moves coming from SAN are committed by using Chess.play()

    // wait for opponent wrapper to finish
    let opponent_wrapper_output = opponent_wrapper_proc.wait().unwrap();
    info!("opponent wrapper exited with status {status}", status = opponent_wrapper_output);
}

#[allow(clippy::too_many_lines)]
fn get_rgb(position: &Chess, state: State) -> RGB {
    let color = position.turn();
    let occupied = position.board().occupied();
    let enemies = position.them();
    match state {
        State::Idle => RGB {
            r: Bitboard::EMPTY,
            g: Bitboard::EMPTY,
            b: Bitboard::EMPTY,
        },
        State::FriendlyPU(square) => {
            let mut canmv_to: Bitboard;
            let mut is_promotion: bool = false;
            if position.board().role_at(square).unwrap() == Role::Pawn {
                let shift_direction = if color.is_white() { 1 } else { -1 };
                canmv_to = Bitboard::from_square(square).shift(8 * shift_direction);
                if (square.rank() == Rank::Second && color.is_white()
                    || square.rank() == Rank::Seventh && color.is_black())
                    && canmv_to.without(occupied).any()
                {
                    canmv_to =
                        canmv_to.with(Bitboard::from_square(square).shift(16 * shift_direction));
                }
                canmv_to = canmv_to.without(occupied);

                if (square.rank() == Rank::Second && color.is_black()
                    || square.rank() == Rank::Seventh && color.is_white())
                    && canmv_to.without(occupied).any()
                {
                    is_promotion = true;
                }
            } else {
                canmv_to = position.board().attacks_from(square).without(occupied);
            }

            let can_capture = position.board().attacks_from(square).intersect(enemies);

            if is_promotion {
                RGB {
                    r: canmv_to.with(can_capture),
                    g: can_capture,
                    b: canmv_to,
                }
            } else {
                RGB {
                    r: can_capture,
                    g: canmv_to.with(can_capture),
                    b: Bitboard::EMPTY,
                }
            }
        }
        State::EnemyPU(square) => {
            let attackers = position.board().attacks_to(square, color, occupied);
            RGB {
                r: Bitboard::EMPTY,
                g: attackers,
                b: Bitboard::EMPTY,
            }
        }
        State::FriendlyAndEnemyPU(_, enemy_square) => RGB {
            r: Bitboard::EMPTY,
            g: Bitboard::from_square(enemy_square),
            b: Bitboard::EMPTY,
        },
        State::Castling(_, rook_square) => {
            let target_square = match (color, rook_square) {
                (Color::White, Square::A1) => Square::C1,
                (Color::White, _) => Square::G1,
                (Color::Black, Square::A8) => Square::C8,
                (Color::Black, _) => Square::G8,
            };

            RGB {
                r: Bitboard::from_square(target_square),
                g: Bitboard::EMPTY,
                b: Bitboard::from_square(target_square),
            }
        }
        State::CastlingPutRookDown(_, _, target_square) => RGB {
            r: Bitboard::from_square(target_square),
            g: Bitboard::EMPTY,
            b: Bitboard::from_square(target_square),
        },
        State::InvalidPiecePU(_, square) | State::InvalidMove(_, square) => RGB {
            r: Bitboard::from_square(square),
            g: Bitboard::EMPTY,
            b: Bitboard::EMPTY,
        },
        State::Error => RGB {
            r: Bitboard::FULL,
            g: Bitboard::EMPTY,
            b: Bitboard::EMPTY,
        },
    }
}

#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RGB {
    r: Bitboard,
    g: Bitboard,
    b: Bitboard,
}

fn print_rgb(rgb: RGB) {
    print_bitboard(rgb.r);
    print_bitboard(rgb.g);
    print_bitboard(rgb.b);
}

#[allow(clippy::too_many_lines, clippy::cognitive_complexity)]
fn update_state(position: &Chess, instruction: u32, state: State) -> (State, Option<Move>) {
    let color = position.turn();
    let square = Square::new(instruction);
    let occupied = position.board().occupied();
    let friendlies = position.us();
    let enemies = position.them();

    match state {
        State::Idle => {
            if friendlies.contains(square) {
                (State::FriendlyPU(square), None)
            } else if enemies.contains(square) {
                if position.board().attacks_to(square, color, occupied).any() {
                    (State::EnemyPU(square), None)
                } else {
                    (State::InvalidPiecePU(None, square), None)
                }
            } else {
                (State::Error, None)
            }
        }
        State::FriendlyPU(prev_square) => {
            let role_picked_up = position.board().role_at(prev_square).unwrap();
            let can_capture = position
                .board()
                .attacks_from(prev_square)
                .intersect(enemies);
            if prev_square == square {
                (State::Idle, None)
            } else if role_picked_up == Role::Rook
                && position.board().role_at(square).is_some()
                && position.board().role_at(square).unwrap() == Role::King
            {
                //castling
                let mv = Move::Castle {
                    king: square,
                    rook: prev_square,
                };
                if position.is_legal(&mv) {
                    (State::Castling(square, prev_square), None)
                } else {
                    (State::InvalidPiecePU(Some(prev_square), square), None)
                }
            } else if role_picked_up == Role::King
                && position.board().role_at(square).is_some()
                && position.board().role_at(square).unwrap() == Role::Rook
            {
                //castling
                let mv = Move::Castle {
                    king: prev_square,
                    rook: square,
                };
                if position.is_legal(&mv) {
                    (State::Castling(prev_square, square), None)
                } else {
                    (State::InvalidPiecePU(Some(prev_square), square), None)
                }
            } else if friendlies.contains(square)
                || (enemies.contains(square) && !can_capture.contains(square))
            {
                (State::InvalidPiecePU(Some(prev_square), square), None)
            } else if can_capture.contains(square) {
                (State::FriendlyAndEnemyPU(prev_square, square), None)
            } else if role_picked_up == Role::Pawn
                && (square.rank() == Rank::First || square.rank() == Rank::Eighth)
            {
                //promotions
                let mv = Move::Normal {
                    role: (Role::Pawn),
                    from: (prev_square),
                    capture: (None),
                    to: (square),
                    promotion: (Some(Role::Queen)),
                }; //Right now we're just assuming the player will promote to queen
                println!("PROMOTED");
                (State::Idle, Some(mv))
            } else {
                let mv = Move::Normal {
                    role: (role_picked_up),
                    from: (prev_square),
                    capture: (None),
                    to: (square),
                    promotion: (None),
                };
                if position.is_legal(&mv) {
                    info!("MOVE COMMITTED");
                    (State::Idle, Some(mv))
                } else {
                    (State::InvalidMove(prev_square, square), None)
                }
            }
        }
        State::EnemyPU(prev_square) => {
            if prev_square == square {
                (State::Idle, None)
            } else if !position
                .board()
                .attacks_to(prev_square, color, occupied)
                .contains(square)
                || enemies.contains(square)
                || (position.board().role_at(square).unwrap() == Role::King
                    && position
                        .king_attackers(prev_square, color.other(), occupied)
                        .any())
            {
                (State::InvalidPiecePU(Some(prev_square), square), None)
            } else if position
                .board()
                .attacks_to(prev_square, color, occupied)
                .contains(square)
            {
                (State::FriendlyAndEnemyPU(square, prev_square), None)
            } else {
                (State::Error, None)
            }
        }
        State::FriendlyAndEnemyPU(prev_friendly_square, prev_enemy_square) => {
            let role_picked_up = position.board().role_at(prev_friendly_square).unwrap();
            if square == prev_friendly_square {
                (State::EnemyPU(prev_enemy_square), None)
            } else if square == prev_enemy_square {
                println!("CAPTURED");
                if role_picked_up == Role::Pawn
                    && (square.rank() == Rank::First || square.rank() == Rank::Eighth)
                {
                    println!("PROMOTED");
                    let mv = Move::Normal {
                        role: (role_picked_up),
                        from: (prev_friendly_square),
                        capture: (position.board().role_at(prev_enemy_square)),
                        to: (square),
                        promotion: (Some(Role::Queen)),
                    }; //assuming player will pick queen
                    (State::Idle, Some(mv))
                } else {
                    let mv = Move::Normal {
                        role: (role_picked_up),
                        from: (prev_friendly_square),
                        capture: (position.board().role_at(prev_enemy_square)),
                        to: (square),
                        promotion: (None),
                    };
                    (State::Idle, Some(mv))
                }
            } else {
                (State::Error, None)
            }
        }
        State::Castling(king_square, rook_square) =>
        //make it more robust
        {
            match color {
                Color::White => {
                    if rook_square.file() == File::A {
                        //queen side
                        if square == Square::C1 {
                            (
                                State::CastlingPutRookDown(king_square, rook_square, Square::D1),
                                None,
                            )
                        } else {
                            (State::Error, None)
                        }
                    } else {
                        //king side
                        if square == Square::G1 {
                            (
                                State::CastlingPutRookDown(king_square, rook_square, Square::F1),
                                None,
                            )
                        } else {
                            (State::Error, None)
                        }
                    }
                }
                Color::Black => {
                    if rook_square.file() == File::A {
                        //queen side
                        if square == Square::C8 {
                            (
                                State::CastlingPutRookDown(king_square, rook_square, Square::D8),
                                None,
                            )
                        } else {
                            (State::Error, None)
                        }
                    } else {
                        //king side
                        if square == Square::G8 {
                            (
                                State::CastlingPutRookDown(king_square, rook_square, Square::F8),
                                None,
                            )
                        } else {
                            (State::Error, None)
                        }
                    }
                }
            }
        }
        State::CastlingPutRookDown(king_square, rook_square, target_square) => {
            if square == target_square {
                let mv = Move::Castle {
                    king: king_square,
                    rook: rook_square,
                };
                (State::Idle, Some(mv))
            } else {
                (State::Error, None)
            }
        }
        State::InvalidPiecePU(prev_prev_square, prev_square) => {
            if square == prev_square && prev_prev_square.is_none() {
                (State::Idle, None)
            } else if square == prev_square && friendlies.contains(prev_prev_square.unwrap()) {
                (State::FriendlyPU(prev_prev_square.unwrap()), None)
            } else if square == prev_square && enemies.contains(prev_prev_square.unwrap()) {
                (State::EnemyPU(prev_prev_square.unwrap()), None)
            } else {
                (State::Error, None)
            }
        }
        State::InvalidMove(prev_prev_square, prev_square) => {
            if square == prev_square {
                (State::FriendlyPU(prev_prev_square), None)
            } else {
                (State::Error, None)
            }
        }
        State::Error => (State::Error, None),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum State {
    Idle,
    FriendlyPU(Square),
    EnemyPU(Square),
    FriendlyAndEnemyPU(Square, Square),
    Castling(Square, Square),
    CastlingPutRookDown(Square, Square, Square),
    InvalidPiecePU(Option<Square>, Square),
    InvalidMove(Square, Square),
    Error,
}

fn print_state_name(state: State) {
    match state {
        State::Idle => println!("Idle"),
        State::FriendlyPU(_) => println!("FriendlyPU"),
        State::EnemyPU(_) => println!("EnemyPU"),
        State::FriendlyAndEnemyPU(_, _) => println!("FriendlyAndEnemyPU"),
        State::Castling(_, _) => println!("Castling"),
        State::CastlingPutRookDown(_, _, _) => println!("CastlingPutRookDown"),
        State::InvalidPiecePU(_, _) => println!("InvalidPiecePU"),
        State::InvalidMove(_, _) => println!("InvalidMove"),
        State::Error => println!("Error"),
    }
}

fn print_board_from_fen(fen: &str) {
    use std::fmt::Write;
    let mut output: String = String::new();
    let mut counter = 0;
    output.push(' ');
    for c in fen.chars() {
        if counter == 8 {
            counter = 0;
            output.push('\n');
        }
        match c {
            c @ ('r' | 'R' | 'n' | 'N' | 'b' | 'B' | 'q' | 'Q' | 'k' | 'K' | 'p' | 'P') => {
                write!(output, "{c} ").unwrap();
                counter += 1;
            }
            n @ '1'..='8' => {
                let n = n.to_digit(10).unwrap();
                for _ in 0..n {
                    output.push_str(". ");
                }
                counter += n;
            }
            _ => {
                output.push(' ');
                counter += 0;
            }
        }
    }
    println!("{output}");
}

fn print_bitboard(bitboard: Bitboard) {
    let y = format!("{bitboard:064b}");

    let mut output: String = String::new();
    let mut line = String::new();
    for (counter, a) in y.chars().enumerate() {
        if counter % 8 == 0 {
            output.push_str(line.chars().rev().collect::<String>().as_str());
            //print!("{}", line.as_str());
            output.push('\n');
            line = String::new();
        }
        line.push(a);
        line.push(' ');
    }
    output.push_str(line.chars().rev().collect::<String>().as_str());
    println!("{}", output.as_str());
}

#[allow(clippy::too_many_lines, clippy::needless_pass_by_value)]
fn move_to_steps(
    mv: Move,
    current_color: Color,
    captured_whites: f64,
    captured_blacks: f64,
) -> Vec<Step> {
    #![allow(clippy::similar_names)]
    let mut steps = Vec::new();

    let from_x: f64 = file_to_float(mv.from().unwrap().file());
    let from_y: f64 = rank_to_float(mv.from().unwrap().rank());
    let to_x: f64 = file_to_float(mv.to().file());
    let to_y: f64 = rank_to_float(mv.to().rank());

    if mv.is_castle() {
        //from = king, to = rook
        let direction = if current_color == Color::White {
            -0.5
        } else {
            0.5
        };
        let (offset, queenside_king) = if (to_x - 8.0).abs() < f64::EPSILON {
            (-1.0, 0.0)
        } else {
            (1.0, 1.0)
        }; // king side castling; else queen side castling
        steps.push(Step {
            x: from_x,
            y: from_y,
            magnet: false,
        });

        steps.push(Step {
            x: to_x + offset + queenside_king,
            y: to_y,
            magnet: true,
        });

        steps.push(Step {
            x: to_x,
            y: to_y,
            magnet: false,
        });

        steps.push(Step {
            x: to_x,
            y: to_y + direction,
            magnet: true,
        });

        steps.push(Step {
            x: from_x - offset,
            y: to_y + direction,
            magnet: true,
        });

        steps.push(Step {
            x: from_x - offset,
            y: from_y,
            magnet: true,
        });

        return steps;
    }

    if mv.is_en_passant() {
        let offset = if current_color == Color::White {
            -1.0
        } else {
            1.0
        };
        let mut capturemvs: Vec<Step> = capture_piece(
            to_x,
            to_y + offset,
            current_color,
            captured_whites,
            captured_blacks,
        );
        steps.append(&mut capturemvs);
    }

    if mv.is_capture() && !mv.is_en_passant() {
        let mut capturemvs: Vec<Step> =
            capture_piece(to_x, to_y, current_color, captured_whites, captured_blacks);
        steps.append(&mut capturemvs);
    }

    let engage: Step = Step {
        x: from_x,
        y: from_y,
        magnet: false,
    };

    steps.push(engage);

    if mv.role() == Role::Knight {
        let step1: Step = Step {
            x: (from_x + to_x) / 2.0,
            y: from_y,
            magnet: true,
        };
        let step2: Step = Step {
            x: (from_x + to_x) / 2.0,
            y: to_y,
            magnet: true,
        };
        let step3: Step = Step {
            x: to_x,
            y: to_y,
            magnet: true,
        };

        steps.push(step1);
        steps.push(step2);
        steps.push(step3);
    }
    //move to position
    else {
        let step: Step = Step {
            x: to_x,
            y: to_y,
            magnet: true,
        };
        steps.push(step);
    }

    steps
}

fn capture_piece(
    from_x: f64,
    from_y: f64,
    current_color: Color,
    captured_whites: f64,
    captured_blacks: f64,
) -> Vec<Step> {
    let mut steps: Vec<Step> = Vec::new();
    steps.push(Step {
        x: from_x,
        y: from_y,
        magnet: false,
    });
    let direction: f64;

    if current_color == Color::White {
        //BLACK IS CAPTURED
        if captured_blacks / 2.0 < from_y {
            direction = -0.5;
        } else {
            direction = 0.5;
        }

        steps.push(Step {
            x: from_x,
            y: (from_y + direction),
            magnet: true,
        });

        steps.push(Step {
            x: (8.5),
            y: (from_y + direction),
            magnet: true,
        });

        steps.push(Step {
            x: (8.5),
            y: (0.5 + captured_blacks / 2.0),
            magnet: true,
        });

        steps.push(Step {
            x: (9.0),
            y: (0.5 + captured_blacks / 2.0),
            magnet: true,
        });
    } else {
        //WHITE IS CAPTURED
        if 8.5 - captured_whites / 2.0 < from_y {
            direction = -0.5;
        } else {
            direction = 0.5;
        }

        steps.push(Step {
            x: from_x,
            y: (from_y + direction),
            magnet: true,
        });

        steps.push(Step {
            x: (0.5),
            y: (from_y + direction),
            magnet: true,
        });

        steps.push(Step {
            x: (0.5),
            y: (8.5 - captured_whites / 2.0),
            magnet: true,
        });

        steps.push(Step {
            x: (0.0),
            y: (8.5 - captured_whites / 2.0),
            magnet: true,
        });
    }

    steps
}

#[derive(Debug, Clone, Copy)]
struct Step {
    x: f64,
    y: f64,
    magnet: bool,
}

fn print_step(step: Step) {
    println!("x: {}", step.x);
    println!("y: {}", step.y);
    println!("magnet: {}", step.magnet);
}

const fn rank_to_float(rank: Rank) -> f64 {
    match rank {
        Rank::First => 1.0,
        Rank::Second => 2.0,
        Rank::Third => 3.0,
        Rank::Fourth => 4.0,
        Rank::Fifth => 5.0,
        Rank::Sixth => 6.0,
        Rank::Seventh => 7.0,
        Rank::Eighth => 8.0,
    }
}

const fn file_to_float(file: File) -> f64 {
    match file {
        File::A => 1.0,
        File::B => 2.0,
        File::C => 3.0,
        File::D => 4.0,
        File::E => 5.0,
        File::F => 6.0,
        File::G => 7.0,
        File::H => 8.0,
    }
}
