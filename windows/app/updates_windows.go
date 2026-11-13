package main

import (
	"os/exec"
	"syscall"
)

func launchUpdateInstaller(path, directory string) error {
	command := exec.Command(path)
	// NSIS requires /D= last and unquoted even when the directory has spaces.
	// This is a process command line, never a shell or file-operation command.
	command.SysProcAttr = &syscall.SysProcAttr{
		HideWindow: true,
		CmdLine:    syscall.EscapeArg(path) + " /S /D=" + directory,
	}
	return command.Start()
}
