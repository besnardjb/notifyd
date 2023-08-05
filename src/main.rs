use clap::Parser;
use tempdir::TempDir;
use std::{path::PathBuf, fs::remove_file};
use which::which;
use std::process::Command;
use std::env;
use std::error::Error;
use std::fmt;
use rocket::State;
use std::sync::{Arc, Mutex, Weak};

/****************
 * DEFINE ERROR *
 ****************/

#[derive(Debug)]
struct NotifydError(String);

impl fmt::Display for NotifydError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "There is an error: {}", self.0)
    }
}

impl Error for NotifydError {}

impl NotifydError {
    fn new(desc : &str) -> Box<dyn Error>
    {
        Box::new(NotifydError(String::from(desc)))
    }
}

/*****************
 * CLI ARGUMENTS *
 *****************/

#[derive(Parser,Debug)]
struct Cli {
    /// The config file to be loaded
    config_file : String
}

/**************
 * TTS ENGINE *
 **************/

#[derive(Debug,PartialEq)]
enum TTSEngine
{
    PICO2WAV,
    ESPEAK,
    ESPEAKNG,
    AUTO
}

struct TtsSentence
{
    text: String,
    path : String
}

impl TtsSentence
{
    fn new(path : &str, text : &str) -> TtsSentence
    {
        TtsSentence{
            text : String::from(text),
            path : String::from(path)
        }
    }

    fn _run_player(self : &Self, player : &str) -> Result<(), Box<dyn std::error::Error>>
    {
        let cmd = [player, self.path.as_str()];

        let ret = Command::new(cmd[0])
        .args(&cmd[1..])
        .output()?;

        if !ret.status.success()
        {
            println!("Failed to run {:?}", cmd);
            println!("{}", String::from_utf8(ret.stderr).unwrap());
            return Err(NotifydError::new("Failed to run tts"));
        }
        Ok(())
    }

    fn play(self : &Self) -> Result<(), Box<dyn std::error::Error>>
    {
        let candidate_players = ["paplay", "mplayer", "play" /* sox */];

        for p in candidate_players
        {
            match which(p) {
                Ok(p) => {
                    match self._run_player(p.to_str().unwrap())
                    {
                        Ok(_) => return Ok(()),
                        Err(e) => return Err(e)
                    }
                },
                Err(_) => {}
            }
        }

        Err(NotifydError::new(format!("Could not find any player in {:?} to play {}", candidate_players, self.path).as_str()))
    }
}

impl Drop for TtsSentence
{
    fn drop(&mut self)
    {
        println!("Removing data for {} : '{}'", self.path, self.text);
        let _ = remove_file(&self.path);
    }
}


struct TTS
{
    enginepath : String,
    lang : String,
    tmpdir : TempDir,
    counter : u64
}

impl TTS
{
    fn _tts_to_bin_name( engine : & TTSEngine) -> &'static str
    {
        match engine {
            TTSEngine::PICO2WAV => "pico2wave",
            TTSEngine::ESPEAK => "espeak",
            TTSEngine::ESPEAKNG =>  "espeak-ng",
            TTSEngine::AUTO => panic!("AUTO engine cannot be instanciated")
        }
    }

    fn _look_for_candidate_engine(engine : TTSEngine) -> Result<TTSEngine, Box<dyn std::error::Error>>
    {
        if engine != TTSEngine::AUTO
        {
            return Ok(engine)
        }

        let engines = vec![TTSEngine::PICO2WAV, TTSEngine::ESPEAK, TTSEngine::ESPEAKNG];

        for e in engines{
            match which(TTS::_tts_to_bin_name(&e))
            {
                Ok(_) => return Ok(e),
                Err(_) => {break;},
            }
        }

        panic!("Cannot find any binary for implementing TTS in PATH");
    }

    fn speak_to_file(self :&mut Self, text : String) -> Result<TtsSentence, Box<dyn std::error::Error>>
    {
        self.counter += 1;
        let outfile = self.tmpdir.path().join(format!("{}.wav",self.counter));
        let outpath: &str = outfile.to_str().expect("Failed to convert path to str");

        let cmd: [&str; 6] = [self.enginepath.as_str(), "-w", outpath, "-l", self.lang.as_str(), text.as_str()];

        let ret = Command::new(cmd[0])
        .args(&cmd[1..])
        .output()?;

        if !ret.status.success()
        {
            println!("{:?}", cmd);
            println!("~~~ Failed to run TSS engine ~~~");
            println!("{}", String::from_utf8(ret.stderr).unwrap());
            println!("~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~");
            return Err(NotifydError::new("Failed to run tts"));
        }

        Ok(TtsSentence::new(outpath, text.as_str()))
    }

    fn get_locale_from_env() -> String
    {
        match env::var("LANG")
        {
            Ok(l) => {
                if l.find(".").is_some()
                {
                    match l.split(".").next()
                    {
                        Some(e) => return e.to_string().replace("_", "-"),
                        None => panic!("Failed to split on '.'")
                    }
                }
                return l.replace("_", "-")
            },
            Err(_) => return String::from("en-US")
        }
    }


    fn new(engine : TTSEngine) -> Result<TTS, Box<dyn std::error::Error>>
    {
        let tmp_dir: TempDir = TempDir::new("notifydtts")?;

        let engine_to_use = TTS::_look_for_candidate_engine(engine)?;

        let engine_binary_name = String::from(TTS::_tts_to_bin_name(&engine_to_use));

        let enginepath : PathBuf;

        match which(&engine_binary_name)
        {
            Ok(path) => enginepath = path,
            Err(_) => panic!("Cannot find TTS engine {} in PATH", engine_binary_name)
        }

        let locale = TTS::get_locale_from_env();

        println!("Using TTS engine {}", engine_binary_name);

        return Ok(TTS { tmpdir: tmp_dir,
                        lang : locale,
                        enginepath: String::from(enginepath.to_string_lossy()),
                        counter : 0 })
    }

     fn clear(self) -> Result<(), Box<dyn std::error::Error>>
    {
        self.tmpdir.close()?;
        Ok(())
    }

}


#[macro_use] extern crate rocket;

#[get("/hello/<name>")]
fn hello(name: &str, tts: &State<TTS>) -> String {
    let say = tts.speak_to_file(name.to_string()).expect("Success");
    say.play();
    name.to_string()
}
#[get("/")]
fn index() -> &'static str {
    "Hello, world!"
}

#[rocket::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {

    let args = Cli::parse();

    let tts = TTS::new(TTSEngine::PICO2WAV)?;

    let _rocket = rocket::build()
        .mount("/", routes![hello])
        .mount("/", routes![index])
        .launch()
        .await?;

    Ok(())
}
