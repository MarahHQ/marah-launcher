//! [`Programs`]: the programs a plugin may launch, found and run briefly. The same rules as a
//! launch apply: `permissions.process`, the folder grants and their symlink checks.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use launcher_api::{
    Location, MAX_PROGRAM_OUTPUT, MAX_PROGRAM_TIME, Permissions, PluginError, PluginId, PluginResult, Programs,
};

use crate::ports::{ProcessSpawner, SpawnCommand};
use crate::{ConfigService, DirStorage};

#[derive(Clone)]
pub(crate) struct HostPrograms {
    pub plugin: PluginId,
    pub permissions: Permissions,
    pub config: Arc<ConfigService>,
    pub home: Option<PathBuf>,
    pub platform: Option<launcher_api::Platform>,
    pub data_dir: PathBuf,
    pub spawner: Arc<dyn ProcessSpawner>,
}

impl HostPrograms {
    /// The real path, after the permission check. `None` when it isn't there.
    fn locate(&self, program: &Location) -> PluginResult<Option<PathBuf>> {
        if !self.permissions.allows_program(program) {
            return Err(PluginError::PermissionDenied(format!("{} may not run {program}", self.plugin)));
        }
        let path = match program {
            Location::Data(path) => DirStorage::new(self.data_dir.clone()).resolve(path)?,
            Location::Folder(path) => {
                match crate::folders::resolve(
                    &self.config,
                    self.home.as_deref(),
                    self.platform,
                    &self.permissions,
                    path,
                ) {
                    Ok(path) => path,
                    Err(PluginError::NotFound(_)) => return Ok(None),
                    Err(e) => return Err(e),
                }
            }
            Location::System(name) => return Ok(self.spawner.which(name)),
        };
        Ok(path.is_file().then_some(path))
    }

    /// [`HostPrograms::locate`] on a blocking thread: a search of `PATH` and the checks of the
    /// folder grants read the file system.
    async fn find_path(&self, program: &Location) -> PluginResult<Option<PathBuf>> {
        let (me, program) = (self.clone(), program.clone());
        let found = tokio::task::spawn_blocking(move || me.locate(&program)).await;
        found.map_err(|e| PluginError::Storage(format!("the program wasn't looked for: {e}")))?
    }
}

#[async_trait]
impl Programs for HostPrograms {
    async fn find(&self, program: &Location) -> PluginResult<Option<String>> {
        Ok(self.find_path(program).await?.map(|p| p.display().to_string()))
    }

    async fn output(&self, program: &Location, args: &[String], timeout: Duration) -> PluginResult<Vec<u8>> {
        let path =
            self.find_path(program).await?.ok_or_else(|| PluginError::NotFound(format!("{program} isn't there")))?;
        let cwd = path.parent().map(ToOwned::to_owned).unwrap_or_else(|| self.data_dir.clone());
        let timeout = timeout.min(MAX_PROGRAM_TIME);
        let command = SpawnCommand {
            program: path,
            args: args.to_vec(),
            env: Default::default(),
            cwd,
            log: None,
            keep_running: false,
        };
        match self.spawner.output(command, timeout, MAX_PROGRAM_OUTPUT).await? {
            Some(mut out) => {
                out.truncate(MAX_PROGRAM_OUTPUT);
                Ok(out)
            }
            None => Err(PluginError::Timeout(timeout.as_millis() as u64)),
        }
    }

    async fn stream(
        &self,
        program: &Location,
        args: &[String],
        elevated: bool,
        line: &(dyn for<'a> Fn(&'a str) + Send + Sync),
        cancelled: &(dyn Fn() -> bool + Send + Sync),
    ) -> PluginResult<i32> {
        if !self.permissions.allows_long_running(program) {
            return Err(PluginError::PermissionDenied(format!("{} may not run {program} long", self.plugin)));
        }
        if elevated && !self.permissions.allows_polkit(program) {
            return Err(PluginError::PermissionDenied(format!("{} may not run {program} through polkit", self.plugin)));
        }
        let path =
            self.find_path(program).await?.ok_or_else(|| PluginError::NotFound(format!("{program} isn't there")))?;
        let cwd = path.parent().map(ToOwned::to_owned).unwrap_or_else(|| self.data_dir.clone());
        let (program, args) = if elevated {
            let spawner = self.spawner.clone();
            let pkexec = tokio::task::spawn_blocking(move || spawner.which("pkexec")).await.ok().flatten();
            let pkexec = pkexec.ok_or_else(|| PluginError::NotFound("pkexec isn't there".into()))?;
            (pkexec, std::iter::once(path.display().to_string()).chain(args.iter().cloned()).collect())
        } else {
            (path, args.to_vec())
        };
        let command = SpawnCommand { program, args, env: Default::default(), cwd, log: None, keep_running: false };
        self.spawner.stream(command, elevated, line, cancelled).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::ports::ProcessHandle;

    /// Finds every program in /usr/bin and keeps each streamed command, with `elevated`.
    #[derive(Default)]
    struct Streams(Mutex<Vec<(SpawnCommand, bool)>>);

    #[async_trait]
    impl ProcessSpawner for Streams {
        async fn spawn(&self, command: SpawnCommand) -> PluginResult<Box<dyn ProcessHandle>> {
            Err(PluginError::Other(format!("{:?} isn't streamed", command.program)))
        }

        async fn output(
            &self,
            command: SpawnCommand,
            _timeout: Duration,
            _limit: usize,
        ) -> PluginResult<Option<Vec<u8>>> {
            Err(PluginError::Other(format!("{:?} isn't streamed", command.program)))
        }

        fn which(&self, name: &str) -> Option<PathBuf> {
            Some(PathBuf::from("/usr/bin").join(name))
        }

        async fn stream(
            &self,
            command: SpawnCommand,
            elevated: bool,
            _line: &(dyn for<'a> Fn(&'a str) + Send + Sync),
            _cancelled: &(dyn Fn() -> bool + Send + Sync),
        ) -> PluginResult<i32> {
            self.0.lock().unwrap().push((command, elevated));
            Ok(0)
        }
    }

    /// The spawner hears that the program is elevated: it gives that one alone an input, for a
    /// cancel it can't do with a kill.
    #[tokio::test]
    async fn an_elevated_program_runs_through_pkexec_and_the_spawner_knows() {
        let spawner = Arc::new(Streams::default());
        let rules = |names: &[&str]| names.iter().map(|n| format!("system:{n}")).collect::<Vec<_>>();
        let permissions = Permissions {
            process: rules(&["installer", "udisksctl"]),
            long_running: rules(&["installer", "udisksctl"]),
            polkit: rules(&["installer"]),
            ..Permissions::default()
        };
        let programs = HostPrograms {
            plugin: PluginId::new("system").unwrap(),
            permissions,
            config: Arc::new(ConfigService::in_memory()),
            home: None,
            platform: None,
            data_dir: std::env::temp_dir(),
            spawner: spawner.clone(),
        };
        let run = |name: &str, elevated| {
            let programs = programs.clone();
            let program = Location::System(name.into());
            async move { programs.stream(&program, &["/dev/vda".into()], elevated, &|_| {}, &|| false).await }
        };
        run("installer", true).await.unwrap();
        run("udisksctl", false).await.unwrap();

        let streamed: Vec<_> =
            spawner.0.lock().unwrap().iter().map(|(c, e)| (c.program.clone(), c.args.clone(), *e)).collect();
        assert_eq!(
            streamed,
            [
                (PathBuf::from("/usr/bin/pkexec"), vec!["/usr/bin/installer".to_owned(), "/dev/vda".to_owned()], true),
                (PathBuf::from("/usr/bin/udisksctl"), vec!["/dev/vda".to_owned()], false),
            ]
        );
    }
}
