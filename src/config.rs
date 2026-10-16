use anyhow::{Context, Result, bail};

#[derive(Clone, Copy)]
pub enum ValidationMode {
    Auto,
    Enabled,
    Disabled,
}

pub struct Config {
    pub validation: ValidationMode,
    pub frame_limit: Option<u64>,
    pub smoke_test: bool,
}

impl Config {
    pub fn from_args(args: impl Iterator<Item = String>) -> Result<Option<Self>> {
        let mut config = Self {
            validation: ValidationMode::Auto,
            frame_limit: None,
            smoke_test: false,
        };
        let mut args = args;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--validation" => config.validation = ValidationMode::Enabled,
                "--no-validation" => config.validation = ValidationMode::Disabled,
                "--frames" => {
                    let count = args
                        .next()
                        .context("--frames requires a positive integer")?;
                    let count: u64 = count.parse().context("Invalid --frames value")?;
                    if count == 0 {
                        bail!("--frames must be greater than zero");
                    }
                    config.frame_limit = Some(count);
                }
                "--smoke-test" => config.smoke_test = true,
                "--help" | "-h" => {
                    println!(
                        "VulkanUS\n\nUsage: vulkanus [--validation | --no-validation] [--frames N] [--smoke-test]\n\nEscape closes the window. --smoke-test exercises resizing, minimize and restore."
                    );
                    return Ok(None);
                }
                _ => bail!("Unknown argument: {arg}. Use --help for usage."),
            }
        }
        if config.smoke_test {
            if config.frame_limit.is_some_and(|count| count < 120) {
                bail!("--smoke-test needs at least 120 frames");
            }
            config.frame_limit = Some(config.frame_limit.unwrap_or(180));
        }
        Ok(Some(config))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_frame_limits_and_options() {
        for args in [
            vec!["--frames"],
            vec!["--frames", "0"],
            vec!["--frames", "abc"],
            vec!["--unknown"],
            vec!["--smoke-test", "--frames", "20"],
        ] {
            assert!(Config::from_args(args.into_iter().map(str::to_string)).is_err());
        }
    }

    #[test]
    fn smoke_test_has_a_finite_default() {
        let config = Config::from_args(["--smoke-test".into()].into_iter())
            .unwrap()
            .unwrap();
        assert_eq!(config.frame_limit, Some(180));
    }
}
