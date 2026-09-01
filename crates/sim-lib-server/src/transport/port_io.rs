//! `std::io` facade over explicitly bound platform transport ports.
use super::bound_transport_services;
use sim_transport_ports::{Half, Listener, SocketAddress, Stream};
use std::{
    io::{self, Read, Write},
    net::{Shutdown, SocketAddr},
    time::Duration,
};

fn io_error(error: sim_transport_ports::TransportError) -> io::Error {
    io::Error::other(error)
}

pub(crate) struct PortTcpListener(Box<dyn Listener>);
impl PortTcpListener {
    pub(crate) fn bind(address: (&str, u16)) -> io::Result<Self> {
        let ports = bound_transport_services().map_err(io_error)?;
        let target = ports
            .dns
            .resolve(address.0, address.1)
            .map_err(io_error)?
            .into_iter()
            .next()
            .ok_or_else(|| io::Error::other("DNS returned no addresses"))?;
        ports
            .sockets
            .listen_tcp(&target)
            .map(Self)
            .map_err(io_error)
    }
    pub(crate) fn local_addr(&self) -> io::Result<SocketAddr> {
        match self.0.local_address().map_err(io_error)? {
            SocketAddress::Ip { address, port } => Ok((address, port).into()),
        }
    }
    pub(crate) fn set_nonblocking(&self, _: bool) -> io::Result<()> {
        Ok(())
    }
    pub(crate) fn accept(&self) -> io::Result<(PortTcpStream, ())> {
        self.0
            .accept()
            .map_err(io_error)?
            .map(|s| (PortTcpStream(s), ()))
            .ok_or_else(|| io::Error::from(io::ErrorKind::WouldBlock))
    }
}

pub(crate) struct PortTcpStream(pub(crate) Box<dyn Stream>);
impl PortTcpStream {
    pub(crate) fn connect(address: (&str, u16)) -> io::Result<Self> {
        let ports = bound_transport_services().map_err(io_error)?;
        let target = ports
            .dns
            .resolve(address.0, address.1)
            .map_err(io_error)?
            .into_iter()
            .next()
            .ok_or_else(|| io::Error::other("DNS returned no addresses"))?;
        ports
            .sockets
            .connect_tcp(&target)
            .map(Self)
            .map_err(io_error)
    }
    pub(crate) fn set_nodelay(&self, _: bool) -> io::Result<()> {
        Ok(())
    }
    pub(crate) fn set_read_timeout(&self, t: Option<Duration>) -> io::Result<()> {
        self.0.set_read_timeout(t).map_err(io_error)
    }
    pub(crate) fn set_write_timeout(&self, _: Option<Duration>) -> io::Result<()> {
        Ok(())
    }
    pub(crate) fn shutdown(&self, h: Shutdown) -> io::Result<()> {
        self.0
            .shutdown(match h {
                Shutdown::Read => Half::Read,
                Shutdown::Write => Half::Write,
                Shutdown::Both => Half::Both,
            })
            .map_err(io_error)
    }
}
impl Read for PortTcpStream {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        self.0.read(b)
    }
}
impl Write for PortTcpStream {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0.write(b)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}
