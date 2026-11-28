"""Build the pinned CPU runtime; no model weights are downloaded by a build."""
import hashlib,json,os,pathlib,shutil,subprocess,zipfile
ROOT=pathlib.Path(__file__).resolve().parents[1]
def main():
    manifest=json.loads((ROOT/'third_party/manifests/enhancement-runtime.json').read_text(encoding='utf-8'))
    source=ROOT/manifest['sourceDirectory'];archive=ROOT/'.local/dependencies/ggml-v1.1.1.zip'
    archive.parent.mkdir(parents=True,exist_ok=True)
    if not archive.exists():
        aria=os.environ.get('ARIA2_NEXT') or shutil.which('aria2-next')
        if not aria:raise RuntimeError('Set ARIA2_NEXT to the aria2-next executable path')
        subprocess.run([aria,'-d',str(archive.parent),'-o',archive.name,'--auto-file-renaming=false',manifest['url']],check=True)
    if hashlib.sha256(archive.read_bytes()).hexdigest()!=manifest['archiveSha256']:raise RuntimeError('ggml source checksum mismatch')
    with zipfile.ZipFile(archive) as z:
        prefix=z.namelist()[0].split('/')[0]+'/'
        for item in z.infolist():
            relative=pathlib.PurePosixPath(item.filename.removeprefix(prefix))
            if '..' in relative.parts or relative.is_absolute():raise RuntimeError('Unsafe source archive path')
            if item.is_dir() or not relative.parts:continue
            path=source/pathlib.Path(*relative.parts);data=z.read(item)
            if path.exists() and path.read_bytes()!=data:raise RuntimeError('Cached ggml source differs from pinned archive: '+str(relative))
            if not path.exists():path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(data)
    finder=pathlib.Path(os.environ.get('ProgramFiles(x86)',r'C:\Program Files (x86)'))/'Microsoft Visual Studio/Installer/vswhere.exe'
    vs=pathlib.Path(subprocess.check_output([str(finder),'-latest','-prerelease','-products','*','-requires','Microsoft.VisualStudio.Component.VC.Tools.x86.x64','-property','installationPath'],text=True).strip())
    cmake=vs/'Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin/cmake.exe'
    ninja=vs/'Common7/IDE/CommonExtensions/Microsoft/CMake/Ninja/ninja.exe'
    if not cmake.is_file():raise RuntimeError('Visual Studio CMake tools are required')
    work=ROOT/'_temp/enhancement-build';work.mkdir(parents=True,exist_ok=True);build=ROOT/'.local/build/enhancement'
    # All arguments are fixed trusted workspace/toolchain paths, not user text.
    commands=['@echo off',f'call "{vs / "VC/Auxiliary/Build/vcvars64.bat"}" >nul',
        f'"{cmake}" -S "{ROOT / "windows/enhancement"}" -B "{build}" -G Ninja -DCMAKE_MAKE_PROGRAM="{ninja}" -DCMAKE_BUILD_TYPE=Release -DGGML_SOURCE="{source}"',
        'if errorlevel 1 exit /b %errorlevel%',f'"{cmake}" --build "{build}" --parallel 8']
    script=work/'build.cmd';script.write_text('\r\n'.join(commands)+'\r\n',encoding='utf-8')
    subprocess.run(['cmd','/c',str(script)],cwd=work,check=True)
    print(build/'FreeTypeEnhancement.dll')
if __name__=='__main__':main()
