use awedio::Sound;
use clap::{Parser, Subcommand};
use kokoros::tts::koko::{TTSKoko, TTSOpts};
use std::net::{IpAddr, SocketAddr};
use std::{
    fs::{self},
    sync::Arc,
};
use tokio::io::{AsyncBufReadExt, BufReader};

#[derive(Subcommand, Debug)]
enum Mode {
    /// Generate speech for a string of text
    ///
    #[command(alias = "t", long_flag_alias = "text", short_flag_alias = 't')]
    Text {
        /// Text to generate speech for
        #[arg(
            default_value = "Hello, This is Kokoro, your remarkable AI TTS. It's a TTS model with merely 82 million parameters yet delivers incredible audio quality.
                This is one of the top notch Rust based inference models, and I'm sure you'll love it. If you do, please give us a star. Thank you very much.
                As the night falls, I wish you all a peaceful and restful sleep. May your dreams be filled with joy and happiness. Good night, and sweet dreams!"
        )]
        text: String,

        /// Path to output the WAV file to on the filesystem
        #[arg(
            short = 'o',
            long = "output",
            value_name = "OUTPUT_PATH",
            default_value = "tmp/output.wav"
        )]
        save_path: String,
    },

    /// Read from a file path and generate a speech file for each line
    #[command(alias = "f", long_flag_alias = "file", short_flag_alias = 'f')]
    File {
        /// Filesystem path to read lines from
        input_path: String,

        /// Format for the output path of each WAV file, where {line} will be replaced with the line number
        #[arg(
            short = 'o',
            long = "output",
            value_name = "OUTPUT_PATH_FORMAT",
            default_value = "tmp/output_{line}.wav"
        )]
        save_path_format: String,
    },

    /// Continuously read from stdin to generate speech, outputting to stdout, for each line
    #[command(aliases = ["stdio", "stdin", "-"], long_flag_aliases = ["stdio", "stdin"])]
    Stream,

    /// Start an OpenAI-compatible HTTP server
    #[command(name = "openai", alias = "oai", long_flag_aliases = ["oai", "openai"])]
    OpenAI {
        /// IP address to bind to (typically 127.0.0.1 or 0.0.0.0)
        #[arg(long, default_value_t = [0, 0, 0, 0].into())]
        ip: IpAddr,

        /// Port to expose the HTTP server on
        #[arg(long, default_value_t = 3000)]
        port: u16,
    },
}

#[derive(Parser, Debug)]
#[command(name = "kokoros")]
#[command(version = "0.1")]
#[command(author = "Lucas Jin")]
struct Cli {
    /// A language identifier from
    /// https://github.com/espeak-ng/espeak-ng/blob/master/docs/languages.md
    #[arg(
        short = 'l',
        long = "lan",
        value_name = "LANGUAGE",
        default_value = "en-us"
    )]
    lan: String,

    /// Path to the Kokoro v1.0 ONNX model on the filesystem
    #[arg(
        short = 'm',
        long = "model",
        value_name = "MODEL_PATH",
        default_value = "checkpoints/kokoro-v1.0.onnx"
    )]
    model_path: String,

    /// Path to the voices data file on the filesystem
    #[arg(
        short = 'd',
        long = "data",
        value_name = "DATA_PATH",
        default_value = "data/voices-v1.0.bin"
    )]
    data_path: String,

    /// Which single voice to use or voices to combine to serve as the style of speech
    #[arg(
        short = 's',
        long = "style",
        value_name = "STYLE",
        // if users use `af_sarah.4+af_nicole.6` as style name
        // then we blend it, with 0.4*af_sarah + 0.6*af_nicole
        default_value = "af_sarah.4+af_nicole.6"
    )]
    style: String,

    /// Rate of speech, as a coefficient of the default
    /// (i.e. 0.0 to 1.0 is slower than default,
    /// whereas 1.0 and beyond is faster than default)
    #[arg(
        short = 'p',
        long = "speed",
        value_name = "SPEED",
        default_value_t = 1.0
    )]
    speed: f32,

    /// Output audio in mono (as opposed to stereo)
    #[arg(long = "mono", default_value_t = false)]
    mono: bool,

    /// Initial silence duration in tokens
    #[arg(long = "initial-silence", value_name = "INITIAL_SILENCE")]
    initial_silence: Option<usize>,

    #[command(subcommand)]
    mode: Mode,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let Cli {
            lan,
            model_path,
            data_path,
            style,
            speed,
            initial_silence,
            mono,
            mode,
        } = Cli::parse();

        let tts = TTSKoko::new(&model_path, &data_path).await;

        match mode {
            Mode::File {
                input_path,
                save_path_format,
            } => {
                let file_content = fs::read_to_string(input_path)?;
                for (i, line) in file_content.lines().enumerate() {
                    let stripped_line = line.trim();
                    if stripped_line.is_empty() {
                        continue;
                    }

                    let save_path = save_path_format.replace("{line}", &i.to_string());
                    tts.tts(TTSOpts {
                        txt: stripped_line,
                        lan: &lan,
                        style_name: &style,
                        save_path: &save_path,
                        mono,
                        speed,
                        initial_silence,
                    })?;
                }
            }

            Mode::Text { text, save_path } => {
                let s = std::time::Instant::now();
                tts.tts(TTSOpts {
                    txt: &text,
                    lan: &lan,
                    style_name: &style,
                    save_path: &save_path,
                    mono,
                    speed,
                    initial_silence,
                })?;
                println!("Time taken: {:?}", s.elapsed());
                let words_per_second =
                    text.split_whitespace().count() as f32 / s.elapsed().as_secs_f32();
                println!("Words per second: {:.2}", words_per_second);
            }

            Mode::OpenAI { ip, port } => {
                let app = kokoros_openai::create_server(tts).await;
                let addr = SocketAddr::from((ip, port));
                let binding = tokio::net::TcpListener::bind(&addr).await?;
                println!("Starting OpenAI-compatible HTTP server on {addr}");
                kokoros_openai::serve(binding, app.into_make_service()).await?;
            }

            Mode::Stream => {
                let stdin = tokio::io::stdin();
                let reader = BufReader::new(stdin);
                let mut lines = reader.lines();

                eprintln!(
                    "Entering streaming mode. Type text and press Enter. Use Ctrl+D to exit."
                );

                let (mut manager, _backend) = awedio::start()?;

                while let Some(line) = lines.next_line().await? {
                    let stripped_line = line.trim();
                    if stripped_line.is_empty() {
                        continue;
                    }

                    // Process the line and get audio data
                    match tts.tts_raw_audio(&stripped_line, &lan, &style, speed, initial_silence) {
                        Ok(raw_audio) => {
                            // Write the raw audio samples directly
                            let s = Arc::new(
                                raw_audio
                                    .iter()
                                    .map(|x| (x * i16::MAX as f32) as i16)
                                    .collect::<Vec<_>>(),
                            );

                            let (sound, notifier) =
                                awedio::sounds::MemorySound::from_samples(s, 1, 24000)
                                    .with_completion_notifier();

                            manager.play(Box::new(sound));
                            let _ = notifier.recv().unwrap();

                            eprintln!("Audio written to stdout. Ready for another line of text.");
                        }
                        Err(e) => eprintln!("Error processing line: {}", e),
                    }
                }
            }
        }

        Ok(())
    })
}
