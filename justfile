name := 'cosmic-external-osd'
bin-src := 'target/release/' + name
bin-dst := '/usr/bin/' + name
service-src := 'res/io.github.cosmic_utils.ExternalOsd.service'
service-dst := '/usr/share/dbus-1/services/io.github.cosmic_utils.ExternalOsd.service'

default: build-release

build-release:
    cargo build --release

install:
    install -Dm0755 {{ bin-src }} {{ bin-dst }}
    install -Dm0644 {{ service-src }} {{ service-dst }}

uninstall:
    rm -f {{ bin-dst }} {{ service-dst }}
