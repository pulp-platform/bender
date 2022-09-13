#!/usr/bin/env bash

export filename="Dockerfile"
rm -f $filename
touch $filename

echo "FROM $full_tgtname" >> $filename
echo >> $filename
if [ $(echo $full_tgtname | cut -d ':' -f 1) = "centos" ]; then
  echo 'RUN yum group install "Development Tools" -y && yum clean all' >> $filename
fi
if [ $(echo $full_tgtname | cut -d ':' -f 1) = "ubuntu" ]; then
  echo 'RUN apt update && apt -y install build-essential curl' >> $filename
fi
if [ $(echo $full_tgtname | cut -d ':' -f 1) = "fedora" ]; then
  echo 'RUN dnf -y update && dnf -y install @development-tools' >> $filename
fi
if [ $(echo $full_tgtname | cut -d ':' -f 1) = "debian" ]; then
  echo 'RUN apt update && apt -y install build-essential curl gcc make' >> $filename
fi
if [ $(echo $full_tgtname | cut -d ':' -f 1) = "almalinux" ]; then
  echo 'RUN dnf -y update && dnf -y group install "Development Tools"' >> $filename
fi
if [[ $(echo $full_tgtname | cut -d ':' -f 1) == "redhat"* ]]; then
  echo 'RUN dnf -y install gcc' >> $filename
fi
echo >> $filename
echo 'ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo' >> $filename
echo 'ENV PATH $CARGO_HOME/bin:$PATH' >> $filename
echo >> $filename
echo 'RUN mkdir -p "$CARGO_HOME" && mkdir -p "$RUSTUP_HOME" && \' >> $filename
echo '    curl https://sh.rustup.rs -sSf | sh -s -- -y --default-toolchain stable && \' >> $filename
echo '    chmod -R a=rwX $CARGO_HOME' >> $filename
echo >> $filename
echo 'WORKDIR /source' >> $filename

