# Package the existing release executable without rebuilding or stripping it.
%global debug_package %{nil}
%global __os_install_post %{nil}
%global _build_id_links none
%global _binary_payload w1.gzdio

Name: benchup
Version: %{benchup_version}
Release: 1
Summary: 3D modeling for woodworking and furniture design
License: MIT AND Apache-2.0 AND ISC
URL: https://benchup3d.github.io/editor/
Source0: payload.tar
BuildArch: x86_64

# These libraries are loaded dynamically, beyond automatic ELF dependencies.
Requires: libEGL.so.1()(64bit)
Requires: libGL.so.1()(64bit)
Requires: libvulkan.so.1()(64bit)
Requires: libwayland-client.so.0()(64bit)
Requires: libwayland-cursor.so.0()(64bit)
Requires: libwayland-egl.so.1()(64bit)
Requires: libX11.so.6()(64bit)
Requires: libX11-xcb.so.1()(64bit)
Requires: libxcb.so.1()(64bit)
Requires: libXcursor.so.1()(64bit)
Requires: libXi.so.6()(64bit)
Requires: libxkbcommon.so.0()(64bit)
Requires: libxkbcommon-x11.so.0()(64bit)
Requires: libXrandr.so.2()(64bit)
Requires: hicolor-icon-theme
Recommends: ca-certificates
Recommends: shared-mime-info
Recommends: desktop-file-utils

%description
Plan boards, wood joints, and complete assemblies with exact measurements.

%prep
%setup -q -c -T

%install
mkdir -p "%{buildroot}"
tar -xf "%{SOURCE0}" -C "%{buildroot}"

%files
%defattr(-,root,root,-)
/usr/bin/benchup-app
/usr/share/applications/io.github.BenchUp3D.BenchUp.desktop
/usr/share/mime/packages/io.github.BenchUp3D.BenchUp.xml
/usr/share/icons/hicolor/*/apps/io.github.BenchUp3D.BenchUp.*
/usr/share/thumbnailers/benchup.thumbnailer
%doc /usr/share/doc/benchup
%license /usr/share/licenses/benchup
