use clap::Parser;
use tempdir::TempDir;
use std::{path::PathBuf, fs::remove_file};
use which::which;
use std::process::Command;


#[derive(Parser,Debug)]
struct Cli {
    /// The config file to be loaded
    config_file : String
}

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
    engine : TTSEngine,
    enginepath : String,
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
            TTSEngine::ESPEAKNG =>  "easpeak-ng",
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

        let cmd: [&str; 6] = [self.enginepath.as_str(), "-w", outpath, "-l", "fr-FR", text.as_str()];

        Command::new(cmd[0])
        .args(&cmd[1..])
        .output()?;

        Ok(TtsSentence::new(outpath, text.as_str()))
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

        println!("Using TTS engine {}", engine_binary_name);

        return Ok(TTS { engine: engine_to_use,
                        tmpdir: tmp_dir,
                        enginepath: String::from(enginepath.to_string_lossy()),
                        counter : 0 })
    }

     fn clear(self) -> Result<(), Box<dyn std::error::Error>>
    {
        self.tmpdir.close()?;
        Ok(())
    }

}



fn main() -> Result<(), Box<dyn std::error::Error>>{
    let args = Cli::parse();

    let mut tts = TTS::new(TTSEngine::AUTO)?;


    let outspeak = tts.speak_to_file(String::from("Bonjour tout le monde !"))?;

    println!("{}", outspeak.path);


    tts.clear()?;

    Ok(())
}
