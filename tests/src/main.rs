mod map_conversion;
mod parking;
mod physics;
mod runner;
mod sim_completion;
mod sim_determinism;
mod transit;

use structopt::StructOpt;

static LOG_ADAPTER: abstutil::LogAdapter = abstutil::LogAdapter;

fn main() {
    log::set_max_level(log::LevelFilter::Debug);
    log::set_logger(&LOG_ADAPTER).unwrap();

    let mut t = runner::TestRunner::new(runner::Flags::from_args());

    map_conversion::run(t.suite("map_conversion"));
    parking::run(t.suite("parking"));
    physics::run(t.suite("physics"));
    sim_completion::run(t.suite("sim_completion"));
    sim_determinism::run(t.suite("sim_determinism"));
    transit::run(t.suite("transit"));

    t.done();
}
