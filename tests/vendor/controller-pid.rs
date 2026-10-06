pub struct Pid {
    kp: f32,
    ki: f32,
    kd: f32,
    output_max: f32,
    integral_max: f32,
    integral: f32,
    /// `None` before the first `update` and after `reset`.
    last_error: Option<f32>,
}

impl Pid {
    pub const fn new(kp: f32, ki: f32, kd: f32, output_max: f32) -> Self {
        Self {
            kp,
            ki,
            kd,
            output_max,
            integral_max: output_max,
            integral: 0.0,
            last_error: None,
        }
    }

    /// The same PID with its integral clamped to ±`integral_max` instead of ±`output_max`, as
    /// OmniX's PID clamps its integral separately (its `max_iout`). OmniX's gains are per tick:
    /// its ki is this ki times `dt_s`, and its kd this kd divided by `dt_s`.
    pub const fn with_integral_max(self, integral_max: f32) -> Self {
        Self {
            integral_max,
            ..self
        }
    }

    // TODO: take the derivative on the measurement rather than the error, so a target that jumps
    // (recentering, a mode switch) does not kick the output. It changes every tuned loop with a
    // nonzero kd, the velocity loops most (their target is the angle loop's output, and its change
    // is part of what they were tuned on), so it waits for a bench retune.
    pub fn update(&mut self, target: f32, measured: f32, dt_s: f32) -> f32 {
        let error = target - measured;
        self.integral += self.ki * error * dt_s;
        self.integral = self.integral.clamp(-self.integral_max, self.integral_max);
        // Safety fix: no derivative on the first tick. OmniX's PID starts its last error at 0, so
        // its first tick's derivative is the whole error over one tick, a kick of `kd` times the
        // error per `dt_s` (the gimbals' pitch loops: 1000 per tick).
        let derivative = self.last_error.map_or(0.0, |last| (error - last) / dt_s);
        self.last_error = Some(error);
        let output = self.kp * error + self.integral + self.kd * derivative;
        // Safety fix: a NaN anywhere, a reading or a target, would otherwise reach the motor and
        // stay in the integral. OmniX's PID means to clear itself and output 0 here, but its clamp
        // (`fminf`/`fmaxf`) turns NaN into -`max_out` first, so its check never fires.
        if output.is_nan() {
            self.reset();
            return 0.0;
        }
        output.clamp(-self.output_max, self.output_max)
    }

    /// Clears the integral and the last error, so the next `update` starts as if new.
    pub fn reset(&mut self) {
        self.integral = 0.0;
        self.last_error = None;
    }
}
